#[actix_web::test]
async fn my_test() {
    assert!(async { 1 }.await, 1);
}

fn main() {}
