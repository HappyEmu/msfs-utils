use msfs_async_derive::data_definition;

#[data_definition]
struct Invalid {
    #[name = "SIM ON GROUND"]
    #[unit = "Bool"]
    value: bool,
}

fn main() {}
