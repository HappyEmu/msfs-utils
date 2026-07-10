use msfs_async_derive::client_data_definition;

#[client_data_definition]
struct Invalid<T> {
    value: T,
}

fn main() {}
