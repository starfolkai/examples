pub struct Player {
    pub x: f32,
    pub y: f32,
    pub lives: u8,
    pub weapon: Weapon,
    pub facing_right: bool,
    pub jumping: bool,
    pub crouching: bool,
}

pub enum Weapon {
    Default,
    Spread,
    Laser,
    Machine,
    Fireball,
}

impl Player {
    pub fn new() -> Self {
        Player {
            x: 32.0, y: 192.0, lives: 3,
            weapon: Weapon::Default,
            facing_right: true, jumping: false, crouching: false,
        }
    }
}
