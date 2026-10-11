//! Prints the OpenAPI document (used to generate the websites' TypeScript client).

fn main() {
    println!("{}", kabados_api::openapi::spec().to_pretty_json().expect("serializable spec"));
}
