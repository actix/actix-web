use std::error::Error as StdError;

#[tokio::main]
async fn main() -> Result<(), Box<dyn StdError>> {
    env_logger::init_from_env(env_logger::Env::new().default_filter_or("info"));

    // construct request builder
    let client = awc::Client::new();

    // configure request
    let request = client
        .get("https://www.rust-lang.org/")
        .append_header(("User-Agent", "Actix-web"));

    println!("Request: {:?}", request);

    let mut response = request.send().await?;

    // server response head
    println!("Response: {:?}", response);

    // read response body
    let body = response.body().await?;
    println!("Downloaded: {:?} bytes", body.len());

    Ok(())
}
