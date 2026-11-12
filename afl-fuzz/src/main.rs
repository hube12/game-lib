#![allow(dead_code)]
#[macro_use]
extern crate afl;

fn main() {
    dummy();
}

fn dummy() {
    fuzz!(|data: &[u8]| {
        if let Ok(_s) = std::str::from_utf8(data) {}
    });
}