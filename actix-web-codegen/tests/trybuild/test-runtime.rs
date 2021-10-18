#[actix_web::test_rt]
async fn my_test() {
    assert!(async { 1 }.await, 1);
}

fn main() {}
