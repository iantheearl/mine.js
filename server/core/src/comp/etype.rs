use specs::{Component, VecStorage};

#[derive(Default)]
pub struct EType(pub String);

impl EType {
    pub fn new(val: &str) -> Self {
        Self(val.to_owned())
    }
}

impl Component for EType {
    type Storage = VecStorage<Self>;
}
