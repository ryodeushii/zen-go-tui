use anyhow::{bail, Result};

pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("profile name cannot be empty");
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        bail!("profile name may only contain ASCII letters, numbers, '-' and '_'");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_profile_name;

    #[test]
    fn profile_name_rejects_path_like_values() {
        assert!(validate_profile_name("live_vocal-1").is_ok());
        assert!(validate_profile_name("../secret").is_err());
        assert!(validate_profile_name("").is_err());
    }
}
