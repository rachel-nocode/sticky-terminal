use std::collections::VecDeque;

pub(crate) struct ScratchpadState {
    pub(crate) open: bool,
    pub(crate) buffer: String,
    pub(crate) history: VecDeque<String>,
}

impl Default for ScratchpadState {
    fn default() -> Self {
        Self {
            open: false,
            buffer: String::new(),
            history: VecDeque::new(),
        }
    }
}
