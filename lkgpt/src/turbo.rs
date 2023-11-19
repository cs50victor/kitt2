use std::{
    sync::{mpsc::Receiver, Arc},
    time::{Duration, Instant},
};

use anyhow::{bail, Result};

use crate::{
    core::Engine,
    scene::{camera::Camera, scene::Scene},
};

use image::{ImageBuffer, Rgba};
use livekit::webrtc::{
    audio_source::native::NativeAudioSource,
    native::yuv_helper,
    video_frame::{I420Buffer, VideoFrame, VideoRotation},
    video_source::native::NativeVideoSource,
};
use log::{info, warn, error};
use parking_lot::Mutex;
use vulkano::{command_buffer::PrimaryCommandBufferAbstract, sync::GpuFuture};

const PIXEL_SIZE: usize = 4;
const FB_WIDTH: usize = 1920;
const FB_HEIGHT: usize = 1080;

#[derive(Clone)]
struct FrameData {
    image: ImageBuffer<Rgba<u8>, Vec<u8>>,
    framebuffer: Arc<Mutex<Vec<u8>>>,
    video_frame: Arc<Mutex<VideoFrame<I420Buffer>>>,
}

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

    fn get_fps_i32(&self) -> u32 {
        self.target_fps as u32
    }

    pub async fn render(
        &mut self,
        // render_input_receiver: Receiver<String>,
        livekit_rtc_src: NativeVideoSource,
    ) -> Result<()> {
        let (pipeline, framebuffer) = self.scene.new_gfx_pipeline_and_frame_buffer()?;
        let now = Instant::now();
        let rotation_start = now;
        let mut num_of_frames = 0;
        let fps = self.get_fps_i32();

        let mut interval =
            tokio::time::interval(Duration::from_millis(1000 / self.target_fps as u64));

        let [w, h] = self.scene.width_height();
        let default_image = ImageBuffer::<Rgba<u8>, Vec<u8>>::from_raw(w, h, vec![0u8; FB_WIDTH * FB_HEIGHT * 4]).unwrap();
        let mut frame_data = FrameData {
            image: default_image,
            framebuffer: Arc::new(Mutex::new(vec![0u8; FB_WIDTH * FB_HEIGHT * 4])),
            video_frame: Arc::new(Mutex::new(VideoFrame {
                rotation: VideoRotation::VideoRotation0,
                buffer: I420Buffer::new(FB_WIDTH as u32, FB_HEIGHT as u32),
                timestamp_us: 0,
            })),
        };

        loop {
            // if let Some(input) = get_user_input(&render_input_receiver)? {
            //     let input = input.as_str();
            //     if is_exit_cmd(input) {
            //         return Ok(());
            //     }
            //     self.scene.handle_input(input);
            // }

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

            frame_data.image = image;

            if let Err(e) = tokio::task::spawn_blocking({
                let frame_data = frame_data.clone();
                let source = livekit_rtc_src.clone();
                move || {
                    let img_vec = frame_data.image.as_raw();
                    let mut framebuffer = frame_data.framebuffer.lock();
                    let mut video_frame = frame_data.video_frame.lock();
                    let i420_buffer = &mut video_frame.buffer;

                    let (stride_y, stride_u, stride_v) = i420_buffer.strides();
                    let (data_y, data_u, data_v) = i420_buffer.data_mut();

                    framebuffer.clone_from_slice(img_vec);

                    yuv_helper::abgr_to_i420(
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
            }).await{
                error!("Error sending video frame to livekit {e}");
            };

            interval.tick().await;

            // if num_of_frames % fps == 0 {
            //     info!(
            //         "[ Frame time after limiter : {:.3} ms ]",
            //         curr_frame_time.elapsed().as_secs_f64() * 1000.0
            //     );
            // }

            num_of_frames = (num_of_frames + 1) % fps;
        }
    }
}


fn get_user_input(input_revr: &Receiver<String>) -> Result<Option<String>> {
    match input_revr.try_recv() {
        Ok(input) => Ok(Some(input.clone())),
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


impl Drop for Turbo {
    fn drop(&mut self) {
        warn!("DROPPED Turbo");
    }
}

unsafe impl Send for Turbo {}
unsafe impl Sync for Turbo {}
