use bevy::ecs::{
    system::Resource,
    world::{FromWorld, World},
};

/// Allows LLM / Model to Control the Bevy World remotely
#[derive(Resource)]
pub struct WorldControlChannel {
    pub tx: crossbeam_channel::Sender<String>,
    rx: crossbeam_channel::Receiver<String>,
}

impl FromWorld for WorldControlChannel {
    fn from_world(_: &mut World) -> Self {
        let (tx, rx) = crossbeam_channel::unbounded::<String>();
        Self { tx, rx }
    }
}
