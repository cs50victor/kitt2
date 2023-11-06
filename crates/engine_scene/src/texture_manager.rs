use engine_assets::texture::Texture;
use std::collections::HashMap;

#[derive(Debug)]
pub struct OrderedTexturesMap {
    map: HashMap<u8, Texture>,
    keys: Vec<u8>,
}

impl Default for OrderedTexturesMap {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderedTexturesMap {
    pub fn new() -> Self {
        OrderedTexturesMap {
            map: HashMap::new(),
            keys: Vec::new(),
        }
    }

    pub fn insert(&mut self, key: u8, value: Texture) {
        let result = self.map.insert(key, value);
        if result.is_none() {
            self.keys.push(key);
        }
    }

    pub fn get(&self, key: &u8) -> Option<&Texture> {
        self.map.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u8, &Texture)> {
        self.keys
            .iter()
            .filter_map(move |k| self.map.get_key_value(k))
    }

    pub fn len(&self) -> u8 {
        self.map.len() as u8
    }
}
