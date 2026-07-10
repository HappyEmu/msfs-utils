use msfs_async_derive::data_definition;

#[data_definition]
struct Invalid {
    #[name = "RADIO HEIGHT"]
    #[name = "PLANE ALTITUDE"]
    #[unit = "Feet"]
    value: f64,
}

fn main() {}
