#[tokio::test(flavor = "local")]
async fn my_test() {
    assert!(async { 1 }.await, 1);
}

fn main() {}
