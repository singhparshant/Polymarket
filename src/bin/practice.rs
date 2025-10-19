fn main() {
    println!("Hello, world!");
    let arr = [10, 20, 30, 40, 50];
    let str = "Hello, world!";
    for i in str.chars() {
        println!("{}", i);
    }
}

fn add(a: i32, b: i32) -> i32 {
    a + b
}
