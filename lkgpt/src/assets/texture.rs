use std::sync::Arc;

use vulkano::image::{view::ImageView, ImmutableImage};

#[derive(Clone, Debug)]
pub struct Texture {
    pub image_view: Arc<ImageView<ImmutableImage>>,
}
