use std::{
    sync::{mpsc::Receiver, Arc},
    time::{Duration, Instant},
};

use anyhow::{bail, Result};

use engine_core::engine::Engine;
use engine_scene::{camera::Camera, scene::Scene};

use image::{ImageBuffer, Rgba};
use livekit::webrtc::{
    native::yuv_helper,
    video_frame::{native::I420BufferExt, I420Buffer, VideoFrame, VideoRotation},
    video_source::native::NativeVideoSource,
};
use log::{info, warn};
use parking_lot::Mutex;
use vulkano::{command_buffer::PrimaryCommandBufferAbstract, sync::GpuFuture};

pub struct Turbo {
    camera: Camera,
    engine: Engine,
    scene: Scene,
    target_fps: f32,
}

impl Turbo {
    pub fn new() -> Result<Self> {
        let engine = Engine::new()?;
        let scene = Scene::new(engine.get_vkdevice(), engine.gfx_queue_family_index())?;
        let camera = Camera::new().update_aspect_ratio_from_scene(scene.width_height());

        let target_fps = 30.0_f32;

        Ok(Self {
            engine,
            scene,
            camera,
            target_fps,
        })
    }

    pub fn load_basic_scene(mut self) -> Result<Self> {
        let donut_model = self.scene.load_gltf("oreo_donut/scene.gltf")?; // , adamHead/adamHead.gltf
        let donut_node = self.scene.create_node(Some(donut_model));
        self.scene.add_node(donut_node);
        Ok(self)
    }

    pub fn get_scene_width_height(&self) -> [u32; 2] {
        self.scene.width_height()
    }

    pub fn get_fps(&self) -> f32 {
        self.target_fps
    }

    pub async fn render(
        &mut self,
        render_input_receiver: Receiver<&str>,
        livekit_vid_src: NativeVideoSource,
    ) -> Result<()> {
        info!("inside turbo renderer");
        let (pipeline, framebuffer) = self.scene.new_gfx_pipeline_and_frame_buffer()?;
        let now = Instant::now();
        let rotation_start = now;
        let mut curr_frame_time = now;
        let mut num_of_frames = 0;

        let target_frame_time = Duration::from_secs_f32(1.0 / (self.target_fps + 1.5)); // thread::sleep isn't accurate, add 1.5 to fps as an error margin

        loop {
            let input = get_user_input(&render_input_receiver)?;
            if let Some(input) = input {
                if is_exit_cmd(input) {
                    return Ok(());
                }
                self.scene.handle_input(input);
            }

            if num_of_frames == 0 {
                self.scene.print_stats();
            }

            let elapsed_time = rotation_start.elapsed();

            self.camera.update_rotation(
                elapsed_time.as_secs_f64() as f32
                    + elapsed_time.subsec_nanos() as f32 / 20_000_000_000.0,
            );

            self.scene
                .update_camera_subbuffer_allocator(self.camera.format_to_subbuffer_data())?;

            let command_buffer = self.scene.build_primary_cmd_buffer(
                &pipeline,
                framebuffer.clone(),
                self.camera.get_model_matrix(),
            )?;

            let finished = command_buffer.execute(self.engine.get_gfx_queue())?;

            finished.then_signal_fence_and_flush()?.wait(None)?;

            let buffer_content = self.scene.img_buffer_content()?.to_vec();

            let [w, h] = self.scene.width_height();

            let image = match ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, buffer_content) {
                Some(img) => img,
                None => bail!("Failed to create image buffer"),
            };

            vulkan_img_to_lvkt_vid_src(livekit_vid_src.clone(), image).await;

            if num_of_frames % 30 == 0 {
                info!(
                    "[ Frame time before limiter : {:.3} ms ]",
                    curr_frame_time.elapsed().as_secs_f64() * 1000.0
                );
            }

            frame_limiter(&mut curr_frame_time, target_frame_time);

            num_of_frames += 1;
        }
    }
}

fn get_user_input<'s>(input_revr: &Receiver<&'s str>) -> Result<Option<&'s str>> {
    match input_revr.try_recv() {
        Ok(input) => Ok(Some(input)),
        Err(err) => match err {
            std::sync::mpsc::TryRecvError::Empty => Ok(None),
            std::sync::mpsc::TryRecvError::Disconnected => {
                bail!("Render input channel disconnected")
            }
        },
    }
}

fn is_exit_cmd(input: &str) -> bool {
    input == "exit"
        || input == "quit"
        || input == "q"
        || input == "bye"
        || input == "goodbye"
        || input == "ciao"
        || input == "adios"
}

fn frame_limiter(curr_frame_time: &mut Instant, target_frame_time: Duration) {
    // TODO - improve later
    let time_taken_to_render_frame = curr_frame_time.elapsed();
    if time_taken_to_render_frame < target_frame_time {
        // thread can sleep for more time not less, more frames can be shrunk down in video
        let time_diff = round_down(
            target_frame_time.as_secs_f64() - time_taken_to_render_frame.as_secs_f64(),
            2,
        );
        let x = Duration::from_secs_f64(time_diff);
        std::thread::sleep(x);
    }
    *curr_frame_time = Instant::now();
}

fn round_down(num: f64, precision: u32) -> f64 {
    let factor = 10.0_f64.powi(precision as i32);
    (num * factor).floor() / factor
}

struct FrameData {
    image: Arc<ImageBuffer<Rgba<u8>, Vec<u8>>>,
    framebuffer: Arc<Mutex<Vec<u8>>>,
    video_frame: Arc<Mutex<VideoFrame<I420Buffer>>>,
}

async fn vulkan_img_to_lvkt_vid_src(
    rtc_source: NativeVideoSource,
    image: ImageBuffer<Rgba<u8>, Vec<u8>>,
) {
    const PIXEL_SIZE: usize = 4;
    const FB_WIDTH: usize = 1920;
    const FB_HEIGHT: usize = 1080;

    tokio::task::spawn_blocking({
        let frame_data = FrameData {
            image: Arc::new(image),
            framebuffer: Arc::new(Mutex::new(vec![0u8; FB_WIDTH * FB_HEIGHT * 4])),
            video_frame: Arc::new(Mutex::new(VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                buffer: I420Buffer::new(FB_WIDTH as u32, FB_HEIGHT as u32),
                timestamp_us: 0,
            })),
        };

        let source = rtc_source.clone();
        move || {
            let img_vec = frame_data.image.as_raw();
            let mut framebuffer = frame_data.framebuffer.lock();
            let mut video_frame = frame_data.video_frame.lock();
            let i420_buffer = &mut video_frame.buffer;

            let (stride_y, stride_u, stride_v) = i420_buffer.strides();
            let (data_y, data_u, data_v) = i420_buffer.data_mut();

            framebuffer.clone_from_slice(img_vec);

            let _x = yuv_helper::abgr_to_i420(
                &framebuffer,
                (FB_WIDTH * PIXEL_SIZE) as u32,
                data_y,
                stride_y,
                data_u,
                stride_u,
                data_v,
                stride_v,
                FB_WIDTH as i32,
                FB_HEIGHT as i32,
            );

            source.capture_frame(&*video_frame);
        }
    })
    .await
    .unwrap();
}

impl Drop for Turbo {
    fn drop(&mut self) {
        warn!("DROPPED Turbo");
    }
}

unsafe impl Send for Turbo {}
unsafe impl Sync for Turbo {}
