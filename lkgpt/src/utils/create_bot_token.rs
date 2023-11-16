use livekit_api::access_token::{AccessToken, VideoGrants};

pub fn create_bot_token(room_name: String, ai_name: &str) -> anyhow::Result<String> {
    let api_key = std::env::var("LIVEKIT_API_KEY")?;
    let api_secret = std::env::var("LIVEKIT_API_SECRET")?;

    let ttl = std::time::Duration::from_secs(60 * 5); // 10 minutes (in sync with frontend)
    Ok(AccessToken::with_api_key(api_key.as_str(), api_secret.as_str())
        .with_ttl(ttl)
        .with_identity(ai_name)
        .with_name(ai_name)
        .with_grants(VideoGrants {
            room: room_name,
            room_list: true,
            room_join: true,
            room_admin: true,
            can_publish: true,
            room_record: true,
            can_subscribe: true,
            can_publish_data: true,
            can_update_own_metadata: true,
            ..Default::default()
        })
        .to_jwt()?)
}
