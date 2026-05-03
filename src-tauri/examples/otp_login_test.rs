use codex_switcher_lib::otp_login::{run_login, LoginInput};

#[tokio::main]
async fn main() {
    let mut email: Option<String> = None;
    let mut timeout: u64 = 180;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--email" => email = args.next(),
            "--timeout" => timeout = args.next().and_then(|s| s.parse().ok()).unwrap_or(180),
            other => eprintln!("unknown arg: {other}"),
        }
    }
    let email = email.expect("--email <addr> required");
    println!("[otp_login_test] email = {email}, timeout = {timeout}s");

    match run_login(
        LoginInput {
            email,
            otp_timeout_secs: timeout,
        },
        None,
    )
    .await
    {
        Ok(out) => {
            println!("\n=== OK ===");
            println!("email      = {}", out.email);
            println!(
                "access_token (前 24): {}…",
                &out.token.access_token.chars().take(24).collect::<String>()
            );
            println!(
                "refresh_token = {}",
                out.token
                    .refresh_token
                    .as_deref()
                    .map(|s| format!("{}…", &s.chars().take(24).collect::<String>()))
                    .unwrap_or("<none>".into())
            );
            println!(
                "id_token (前 32) = {}",
                out.token
                    .id_token
                    .as_deref()
                    .map(|s| format!("{}…", &s.chars().take(32).collect::<String>()))
                    .unwrap_or("<none>".into())
            );
            println!("expires_in = {:?}", out.token.expires_in);
        }
        Err(e) => {
            eprintln!("\n=== FAIL ===\n{e}");
            std::process::exit(1);
        }
    }
}
