pub struct Helper {
    pub name: String,
}

pub fn make_helper(name: &str) -> Helper {
    Helper {
        name: name.to_owned(),
    }
}
