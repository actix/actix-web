use actix_web::client::Client;

#[actix_rt::main]
async fn main() -> Result<(), actix_web::Error> {
    // std::env::set_var("RUST_LOG", "actix_http=trace");
    let client = Client::default();

    // Create request builder and send request
    let mut response = client
        .get("https://www.rust-lang.org") // <--- notice the "s" in "https://..."
        .header("User-Agent", "Actix-web")
        .send()
        .await?; // <- Send http request
    println!("Response: {:?}", response);
    let body = response.body().await?;
    println!("Downloaded: {:?} bytes", body.len());
    Ok(())
}
