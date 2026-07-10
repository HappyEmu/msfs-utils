use msfs_async_derive::client_data_definition;

#[client_data_definition]
#[repr(C, packed)]
struct Invalid {
    value: u32,
}

fn main() {}
