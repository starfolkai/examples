pub enum EnemyType {
    Soldier,
    Sniper,
    Turret,
    Runner,
    Boss,
}

pub struct Enemy {
    pub enemy_type: EnemyType,
    pub x: f32,
    pub y: f32,
    pub health: u8,
    pub active: bool,
}

impl Enemy {
    pub fn new(enemy_type: EnemyType, x: f32, y: f32) -> Self {
        Enemy { enemy_type, x, y, health: 1, active: true }
    }
}
