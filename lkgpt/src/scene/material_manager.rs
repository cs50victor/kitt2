use crate::assets::materials::MeshMaterial;
use std::collections::HashMap;

#[derive(Debug)]
pub struct OrderedMaterialsMap {
    map: HashMap<u8, MeshMaterial>,
    keys: Vec<u8>,
}

impl Default for OrderedMaterialsMap {
    fn default() -> Self {
        Self::new()
    }
}

impl OrderedMaterialsMap {
    pub fn new() -> Self {
        OrderedMaterialsMap {
            map: HashMap::new(),
            keys: Vec::new(),
        }
    }

    pub fn insert(&mut self, key: u8, value: MeshMaterial) {
        let result = self.map.insert(key, value);
        if result.is_none() {
            self.keys.push(key);
        }
    }

    pub fn get(&self, key: &u8) -> Option<&MeshMaterial> {
        self.map.get(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&u8, &MeshMaterial)> {
        self.keys
            .iter()
            .filter_map(move |k| self.map.get_key_value(k))
    }

    pub fn len(&self) -> u8 {
        self.map.len() as u8
    }
}
