//! Manual probe for OpenRouter's TTS models listing. Run with:
//! `cargo test -p dicto-translate --test openrouter_probe -- --ignored --nocapture`

use dicto_translate::openai::list_tts_models;

#[test]
#[ignore]
fn probe_openrouter_tts_models() {
    let base = "https://openrouter.ai/api/v1";
    match list_tts_models("", base) {
        Ok(models) => {
            println!("got {} tts models", models.len());
            for m in models.iter().take(5) {
                println!("- {} ({} voices)", m.id, m.voices.len());
            }
        }
        Err(e) => {
            println!("ERROR: {e}");
            if let Some(src) = std::error::Error::source(&e) {
                println!("source: {src}");
            }
        }
    }
}

/// Dump the raw bytes + headers of the exact URL we request, to see what
/// the server really sends (encoding, content type, shape).
#[test]
#[ignore]
fn probe_raw_response() {
    let url = "https://openrouter.ai/api/v1/models?output_modalities=speech";
    let resp = reqwest::blocking::get(url).expect("request failed");
    println!("status: {}", resp.status());
    for (k, v) in resp.headers().iter() {
        println!("header: {k}: {:?}", v);
    }
    let bytes = resp.bytes().expect("bytes failed");
    println!("body bytes: {}", bytes.len());
    println!(
        "body head: {}",
        String::from_utf8_lossy(&bytes[..bytes.len().min(300)])
    );
}
