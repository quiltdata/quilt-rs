use quilt_rs::random_string;
use quilt_rs::Rectangle;

fn main() {
    let s = random_string(10);
    println!("Hello, {}!", s);

    let r = Rectangle {
        width: 8,
        height: 7,
    };
    println!("Rectangle: {:?}", r);
}
