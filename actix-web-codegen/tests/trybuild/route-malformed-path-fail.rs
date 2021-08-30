use actix_web_codegen::get;

#[get("/{")]
async fn zero() -> &'static str {
    "malformed resource def"
}

#[get("/{foo")]
async fn one() -> &'static str {
    "malformed resource def"
}

#[get("/{}")]
async fn two() -> &'static str {
    "malformed resource def"
}

#[get("/*")]
async fn three() -> &'static str {
    "malformed resource def"
}

#[get("/{tail:\\d+}*")]
async fn four() -> &'static str {
    "malformed resource def"
}

#[get("/{a}/{b}/{c}/{d}/{e}/{f}/{g}/{h}/{i}/{j}/{k}/{l}/{m}/{n}/{o}/{p}/{q}")]
async fn five() -> &'static str {
    "malformed resource def"
}

fn main() {}
