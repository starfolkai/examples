pub struct Level {
    pub stage: u8,
    pub scroll_x: u16,
    pub scroll_y: u16,
    pub width: u16,
}

impl Level {
    pub fn new(stage: u8) -> Self {
        Level { stage, scroll_x: 0, scroll_y: 0, width: 8192 }
    }
}
