use std::fmt::Debug;

use log::warn;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerMsg<T> {
    data: Option<T>,
    error: Option<String>,
}

impl<T: ToString> ServerMsg<T> {
    pub fn data(data: T) -> Self {
        Self {
            data: Some(data),
            error: None,
        }
    }

    pub fn error(error: T) -> Self {
        let err_msg = error.to_string();
        warn!("Server error. {err_msg:?}");
        Self {
            data: None,
            error: Some(err_msg),
        }
    }
}

#[derive(Serialize)]
pub struct DefaultGameResponse {
    pub board: Vec<i8>,
    pub state: String,
}
#[derive(Deserialize)]
pub struct PlayDetails {
    pub position: u8,
}

pub enum CommonResponses {
    MethodNotAllowed,
}

impl CommonResponses {
    pub fn json(&self) -> ServerMsg<String> {
        match self {
            CommonResponses::MethodNotAllowed => ServerMsg::error("Method not allowed".to_string()),
        }
    }
}
