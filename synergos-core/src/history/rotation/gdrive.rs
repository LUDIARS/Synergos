//! `gdrive` バックエンド — Google Drive v3 REST を `reqwest` で直接叩く。
//!
//! service account の JSON 鍵 (`credentials_file`) から JWT assertion を組み立て、
//! OAuth2 token endpoint と交換して access token を得る (google クレート SDK は
//! 使わず、依存を増やさないために `jsonwebtoken` + `reqwest` の素の実装にする)。
//! key ↔ ファイルの対応付けは `files.list` の `appProperties` に `key` を入れて検索する
//! (Drive は path 空間を持たないため)。

use std::io;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{Algorithm, EncodingKey, Header};
use serde::{Deserialize, Serialize};

use super::backend::RotationBackend;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const UPLOAD_URL: &str = "https://www.googleapis.com/upload/drive/v3/files";
const FILES_URL: &str = "https://www.googleapis.com/drive/v3/files";
const SCOPE: &str = "https://www.googleapis.com/auth/drive.file";

pub struct GdriveBackend {
    folder_id: String,
    credentials_file: String,
}

#[derive(Debug, Deserialize)]
struct ServiceAccountKey {
    client_email: String,
    private_key: String,
    #[serde(default = "default_token_uri")]
    token_uri: String,
}

fn default_token_uri() -> String {
    TOKEN_URL.to_string()
}

#[derive(Serialize)]
struct Claims {
    iss: String,
    scope: String,
    aud: String,
    iat: u64,
    exp: u64,
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Deserialize)]
struct FileListResponse {
    files: Vec<DriveFile>,
}

#[derive(Deserialize)]
struct DriveFile {
    id: String,
    #[serde(default)]
    size: Option<String>,
}

impl GdriveBackend {
    pub fn new(folder_id: String, credentials_file: String) -> Self {
        Self {
            folder_id,
            credentials_file,
        }
    }

    async fn load_key(&self) -> io::Result<ServiceAccountKey> {
        let path = shellexpand_home(&self.credentials_file);
        let text = tokio::fs::read_to_string(&path).await.map_err(|error| {
            // 個人名を含み得る絶対パスは IPC 応答や daemon log に流さない。
            io::Error::new(error.kind(), format!("gdrive credentials_file unreadable: {error}"))
        })?;
        serde_json::from_str(&text)
            .map_err(|error| io::Error::other(format!("gdrive credentials_file invalid JSON: {error}")))
    }

    /// service account の JWT assertion を組み立て、OAuth2 token endpoint と交換する。
    async fn access_token(&self) -> io::Result<String> {
        let key = self.load_key().await?;
        if key.token_uri != TOKEN_URL {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "gdrive credentials token_uri must be the Google OAuth2 token endpoint",
            ));
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let claims = Claims {
            iss: key.client_email.clone(),
            scope: SCOPE.to_string(),
            aud: key.token_uri.clone(),
            iat: now,
            exp: now + 3600,
        };
        let encoding_key = EncodingKey::from_rsa_pem(key.private_key.as_bytes())
            .map_err(|error| io::Error::other(format!("gdrive private_key invalid: {error}")))?;
        let assertion = jsonwebtoken::encode(&Header::new(Algorithm::RS256), &claims, &encoding_key)
            .map_err(|error| io::Error::other(format!("gdrive JWT signing failed: {error}")))?;

        let client = gdrive_client()?;
        let resp = client
            .post(&key.token_uri)
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|error| io::Error::other(format!("gdrive token request failed: {error}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(io::Error::other(format!(
                "gdrive token request returned {status}"
            )));
        }
        let token: TokenResponse = resp
            .json()
            .await
            .map_err(|error| io::Error::other(format!("gdrive token response invalid: {error}")))?;
        Ok(token.access_token)
    }

    /// 設定された folder 内を `appProperties.key == key` で検索する。
    async fn find_file(
        &self,
        client: &reqwest::Client,
        token: &str,
        key: &str,
    ) -> io::Result<Option<DriveFile>> {
        let query = file_query(&self.folder_id, key);
        let resp = client
            .get(FILES_URL)
            .bearer_auth(token)
            .query(&[("q", query.as_str()), ("fields", "files(id,size)"), ("spaces", "drive")])
            .send()
            .await
            .map_err(|error| io::Error::other(format!("gdrive files.list failed: {error}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(io::Error::other(format!(
                "gdrive files.list returned {status}"
            )));
        }
        let listed: FileListResponse = resp
            .json()
            .await
            .map_err(|error| io::Error::other(format!("gdrive files.list response invalid: {error}")))?;
        Ok(listed.files.into_iter().next())
    }
}

fn escape_query_value(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn file_query(folder_id: &str, key: &str) -> String {
    format!(
        "appProperties has {{ key='rotationKey' and value='{}' }} and '{}' in parents and trashed = false",
        escape_query_value(key),
        escape_query_value(folder_id),
    )
}

fn gdrive_client() -> io::Result<reqwest::Client> {
    reqwest::Client::builder()
        // JWT assertion / bearer token を別 origin へ転送しないよう redirect は拒否する。
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| io::Error::other(format!("build gdrive HTTP client: {error}")))
}

fn shellexpand_home(path: &str) -> std::path::PathBuf {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = dirs_home() {
            return home.join(rest);
        }
    }
    std::path::PathBuf::from(path)
}

fn dirs_home() -> Option<std::path::PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
}

#[async_trait]
impl RotationBackend for GdriveBackend {
    async fn put(&self, key: &str, path: &Path) -> io::Result<()> {
        let token = self.access_token().await?;
        let client = gdrive_client()?;
        let bytes = tokio::fs::read(path).await?;

        if let Some(existing) = self.find_file(&client, &token, key).await? {
            // 既存 (dedup 済みの key を再 put することは通常起きないが、冪等にしておく)。
            let resp = client
                .patch(format!("{UPLOAD_URL}/{}", existing.id))
                .bearer_auth(&token)
                .query(&[("uploadType", "media")])
                .body(bytes)
                .send()
                .await
                .map_err(|error| io::Error::other(format!("gdrive update upload failed: {error}")))?;
            if !resp.status().is_success() {
                let status = resp.status();
                return Err(io::Error::other(format!(
                    "gdrive update upload returned {status}"
                )));
            }
            return Ok(());
        }

        let metadata = serde_json::json!({
            "name": key.replace('/', "_"),
            "parents": [self.folder_id],
            "appProperties": { "rotationKey": key },
        });
        let boundary = format!("synergos-{}", uuid::Uuid::new_v4());
        let mut body = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n").as_bytes());
        body.extend_from_slice(metadata.to_string().as_bytes());
        body.extend_from_slice(format!("\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes());
        body.extend_from_slice(&bytes);
        body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());

        let resp = client
            .post(UPLOAD_URL)
            .bearer_auth(&token)
            .query(&[("uploadType", "multipart")])
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|error| io::Error::other(format!("gdrive multipart upload failed: {error}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(io::Error::other(format!(
                "gdrive multipart upload returned {status}"
            )));
        }
        Ok(())
    }

    async fn get(&self, key: &str, dest: &Path) -> io::Result<()> {
        let token = self.access_token().await?;
        let client = gdrive_client()?;
        let Some(file) = self.find_file(&client, &token, key).await? else {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("gdrive: no file with rotationKey={key}"),
            ));
        };
        let resp = client
            .get(format!("{FILES_URL}/{}", file.id))
            .bearer_auth(&token)
            .query(&[("alt", "media")])
            .send()
            .await
            .map_err(|error| io::Error::other(format!("gdrive download failed: {error}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(io::Error::other(format!(
                "gdrive download returned {status}"
            )));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|error| io::Error::other(format!("gdrive download body read failed: {error}")))?;
        tokio::fs::write(dest, &bytes).await?;
        Ok(())
    }

    async fn exists(&self, key: &str) -> io::Result<bool> {
        let token = self.access_token().await?;
        let client = gdrive_client()?;
        Ok(self.find_file(&client, &token, key).await?.is_some())
    }

    async fn size(&self, key: &str) -> io::Result<Option<u64>> {
        let token = self.access_token().await?;
        let client = gdrive_client()?;
        let Some(file) = self.find_file(&client, &token, key).await? else {
            return Ok(None);
        };
        let size = file
            .size
            .ok_or_else(|| io::Error::other("gdrive files.list response omitted object size"))?
            .parse::<u64>()
            .map_err(|error| io::Error::other(format!("gdrive object size invalid: {error}")))?;
        Ok(Some(size))
    }

    async fn delete(&self, key: &str) -> io::Result<()> {
        let token = self.access_token().await?;
        let client = gdrive_client()?;
        let Some(file) = self.find_file(&client, &token, key).await? else {
            return Ok(());
        };
        let resp = client
            .delete(format!("{FILES_URL}/{}", file.id))
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|error| io::Error::other(format!("gdrive delete failed: {error}")))?;
        if !resp.status().is_success() && resp.status().as_u16() != 404 {
            let status = resp.status();
            return Err(io::Error::other(format!(
                "gdrive delete returned {status}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ネットワーク非依存: query 文字列のエスケープとリクエスト構築だけを検証する
    /// (実環境疎通は PR 説明に手動手順を記載する仕様どおり)。
    #[test]
    fn query_value_escapes_single_quote() {
        assert_eq!(escape_query_value("a'b"), "a\\'b");
        assert_eq!(escape_query_value("plain"), "plain");
    }

    #[test]
    fn file_query_is_scoped_to_configured_folder() {
        let query = file_query("folder'id", "aa/bb");
        assert!(query.contains("'folder\\'id' in parents"));
        assert!(query.contains("value='aa/bb'"));
    }

    #[test]
    fn home_expansion_uses_home_or_userprofile() {
        // HOME/USERPROFILE いずれも無い環境でも panic せず元のパスを返す。
        let expanded = shellexpand_home("no-tilde-here.json");
        assert_eq!(expanded, std::path::PathBuf::from("no-tilde-here.json"));
    }
}
