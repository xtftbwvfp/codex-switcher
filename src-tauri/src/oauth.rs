use base64::{engine::general_purpose, Engine as _};
use rand::{rng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// OpenAI 官方授权常量 (参考 codex-main)
pub const CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
pub const AUTH_URL: &str = "https://auth.openai.com/oauth/authorize";
pub const TOKEN_URL: &str = "https://auth.openai.com/oauth/token";

/// PKCE 相关的代码
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PkceCodes {
    pub code_verifier: String,
    pub code_challenge: String,
}

/// 生成 PKCE 代码对 (与官方一致: 64字节)
pub fn generate_pkce() -> PkceCodes {
    let mut bytes = [0u8; 64];
    rng().fill_bytes(&mut bytes);

    // 生成 verifier (Base64URL 编码)
    let code_verifier = general_purpose::URL_SAFE_NO_PAD.encode(bytes);

    // 生成 challenge (SHA256 哈希后 Base64URL 编码)
    let mut hasher = Sha256::new();
    hasher.update(code_verifier.as_bytes());
    let code_challenge = general_purpose::URL_SAFE_NO_PAD.encode(hasher.finalize());

    PkceCodes {
        code_verifier,
        code_challenge,
    }
}

/// 令牌响应结构
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub id_token: Option<String>,
    pub expires_in: Option<u64>,
}

/// 用户信息预提取 (通过解析 id_token)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserInfo {
    pub email: String,
    pub account_id: Option<String>,
}

/// 使用授权码交换访问令牌 (与官方一致: 手动拼接请求体)
pub async fn exchange_code(
    code: &str,
    redirect_uri: &str,
    code_verifier: &str,
) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();

    // 官方格式: 手动拼接字符串
    let body = format!(
        "grant_type=authorization_code&code={}&redirect_uri={}&client_id={}&code_verifier={}",
        urlencoding::encode(code),
        urlencoding::encode(redirect_uri),
        urlencoding::encode(CLIENT_ID),
        urlencoding::encode(code_verifier)
    );

    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
        .send()
        .await
        .map_err(|e| format!("请求令牌失败: {}", e))?;

    if !response.status().is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("OpenAI 返回错误: {}", error_body));
    }

    response
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("解析令牌响应失败: {}", e))
}

/// 使用刷新令牌获取新访问令牌
pub async fn refresh_access_token(refresh_token: &str) -> Result<TokenResponse, String> {
    let client = reqwest::Client::new();

    let params = [
        ("grant_type", "refresh_token"),
        ("client_id", CLIENT_ID),
        ("refresh_token", refresh_token),
        ("scope", "openid profile email offline_access"),
    ];

    let response = client
        .post(TOKEN_URL)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .form(&params)
        .send()
        .await
        .map_err(|e| format!("刷新令牌失败: {}", e))?;

    if !response.status().is_success() {
        let error_body = response.text().await.unwrap_or_default();
        return Err(format!("刷新令牌被拒绝: {}", error_body));
    }

    response
        .json::<TokenResponse>()
        .await
        .map_err(|e| format!("解析刷新响应失败: {}", e))
}

/// 从 ID Token 中提取用户信息 (JWT 解析)
pub fn parse_user_info(id_token: &str) -> Option<UserInfo> {
    let parts: Vec<&str> = id_token.split('.').collect();
    if parts.len() < 2 {
        return None;
    }

    let payload = general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&payload).ok()?;

    let email = json.get("email")?.as_str()?.to_string();

    // 从 OpenAI 特有的 claims 中获取 account_id
    let account_id = json
        .get("https://api.openai.com/auth")
        .and_then(|v| v.get("chatgpt_account_id"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    Some(UserInfo { email, account_id })
}
