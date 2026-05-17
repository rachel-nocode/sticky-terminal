use serde::{Deserialize, Serialize};

use crate::sticky::PaperColor;

#[derive(Serialize, Deserialize, Default)]
pub(crate) struct AppConfig {
    #[serde(default)]
    pub(crate) paper: PaperColor,
}
