use std::error::Error as StdError;

#[actix_web::main]
async fn main() -> Result<(), Box<dyn StdError>> {
    std::env::set_var("RUST_LOG", "client=trace,awc=trace,actix_http=trace");
    env_logger::init();

    let client = awc::Client::new();

    // Create request builder, configure request and send
    let request = client
        .get("https://www.rust-lang.org/")
        .append_header(("User-Agent", "Actix-web"));

    println!("Request: {:?}", request);

    let mut response = request.send().await?;

    // server http response
    println!("Response: {:?}", response);

    // read response body
    let body = response.body().await?;
    println!("Downloaded: {:?} bytes", body.len());

    Ok(())
}
