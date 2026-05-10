use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, thiserror::Error)]
pub enum PrivacyRejection {
    #[error("AWS access key")]
    AwsAccessKey,
    #[error("GitHub personal access token")]
    GitHubPat,
    #[error("GitHub fine-grained personal access token")]
    GitHubFineGrainedPat,
    #[error("JWT")]
    Jwt,
    #[error("OpenAI API key")]
    OpenAiKey,
    #[error("Anthropic API key")]
    AnthropicKey,
    #[error("Stripe live secret key")]
    StripeLiveKey,
    #[error("private key material")]
    PrivateKey,
    #[error("high entropy token")]
    HighEntropy,
    #[error("secrets path")]
    SecretsPath,
    #[error("environment file path")]
    EnvPath,
    #[error("memory ignore marker")]
    MemoryIgnore,
    #[error("SSH path")]
    SshPath,
    #[error("key file path")]
    KeyPath,
    #[error("AWS credentials path")]
    AwsCredentialsPath,
    #[error("config path")]
    ConfigPath,
}

impl PrivacyRejection {
    pub fn telemetry_reason(self) -> &'static str {
        match self {
            Self::AwsAccessKey => "regex:aws",
            Self::GitHubPat => "regex:github-pat",
            Self::GitHubFineGrainedPat => "regex:github-fine-grained-pat",
            Self::Jwt => "regex:jwt",
            Self::OpenAiKey => "regex:openai",
            Self::AnthropicKey => "regex:anthropic",
            Self::StripeLiveKey => "regex:stripe-live",
            Self::PrivateKey => "regex:private-key",
            Self::HighEntropy => "entropy",
            Self::SecretsPath => "path:secrets-dir",
            Self::EnvPath => "path:env",
            Self::MemoryIgnore => "marker:memory-ignore",
            Self::SshPath => "path:ssh",
            Self::KeyPath => "path:key-file",
            Self::AwsCredentialsPath => "path:aws-credentials",
            Self::ConfigPath => "path:config",
        }
    }
}

pub fn privacy_filter_path(path: impl AsRef<Path>) -> Result<(), PrivacyRejection> {
    let path = path.as_ref().to_string_lossy();
    if path.starts_with(".secrets.d/")
        || path.contains("/.secrets.d/")
        || path.starts_with("~/.secrets.d/")
        || path.starts_with(".age/")
        || path.contains("/.age/")
        || path.starts_with(".gnupg/")
        || path.contains("/.gnupg/")
        || path == ".netrc"
        || path.ends_with("/.netrc")
        || path == ".git-credentials"
        || path.ends_with("/.git-credentials")
        || path == "auth.json"
        || path.ends_with("/auth.json")
        || path == ".nuget/NuGet/NuGet.Config"
        || path.ends_with("/.nuget/NuGet/NuGet.Config")
        || path == ".m2/settings.xml"
        || path.ends_with("/.m2/settings.xml")
        || path == ".gradle/gradle.properties"
        || path.ends_with("/.gradle/gradle.properties")
        || path == ".bundle/config"
        || path.ends_with("/.bundle/config")
        || path == ".vault-token"
        || path.ends_with("/.vault-token")
        || path == ".sentryclirc"
        || path.ends_with("/.sentryclirc")
        || path == ".npmrc"
        || path.ends_with("/.npmrc")
        || path == ".pnpmrc"
        || path.ends_with("/.pnpmrc")
        || path == ".yarnrc.yml"
        || path.ends_with("/.yarnrc.yml")
        || path == ".pypirc"
        || path.ends_with("/.pypirc")
        || path == ".gem/credentials"
        || path.ends_with("/.gem/credentials")
        || path == ".cargo/credentials.toml"
        || path.ends_with("/.cargo/credentials.toml")
        || path == ".docker/config.json"
        || path.ends_with("/.docker/config.json")
        || path == ".kube/config"
        || path.ends_with("/.kube/config")
        || path == ".azure/accessTokens.json"
        || path.ends_with("/.azure/accessTokens.json")
        || path == ".azure/msal_token_cache.json"
        || path.ends_with("/.azure/msal_token_cache.json")
        || path.ends_with("application_default_credentials.json")
        || path == ".terraform.d/credentials.tfrc.json"
        || path.ends_with("/.terraform.d/credentials.tfrc.json")
        || path == ".pulumi/credentials.json"
        || path.ends_with("/.pulumi/credentials.json")
        || path == ".oci/config"
        || path.ends_with("/.oci/config")
        || path.ends_with(".kdbx")
        || path.ends_with(".kdb")
    {
        return Err(PrivacyRejection::SecretsPath);
    }
    if path == ".env" || path == ".envrc" || path.starts_with(".env.") || path.contains("/.env") {
        return Err(PrivacyRejection::EnvPath);
    }
    if path.starts_with(".ssh/") || path.contains("/.ssh/") {
        return Err(PrivacyRejection::SshPath);
    }
    if path == ".aws/credentials"
        || path.ends_with("/.aws/credentials")
        || path == ".aws/config"
        || path.ends_with("/.aws/config")
        || path.starts_with(".aws/sso/cache/")
        || path.contains("/.aws/sso/cache/")
    {
        return Err(PrivacyRejection::AwsCredentialsPath);
    }
    if path.starts_with(".config/") || path.contains("/.config/") {
        return Err(PrivacyRejection::ConfigPath);
    }
    if path.ends_with(".pem")
        || path.ends_with(".key")
        || path.ends_with(".p12")
        || path.ends_with(".pfx")
        || path.ends_with(".ppk")
        || path.ends_with(".jks")
        || path.ends_with(".keystore")
    {
        return Err(PrivacyRejection::KeyPath);
    }
    Ok(())
}

pub fn privacy_filter(content: &str) -> Result<(), PrivacyRejection> {
    if content
        .lines()
        .next()
        .is_some_and(|line| line.trim() == "<!-- memory:ignore -->")
        || content.lines().any(|line| line.contains("# memory:ignore"))
    {
        return Err(PrivacyRejection::MemoryIgnore);
    }
    if content.contains("BEGIN PRIVATE KEY")
        || content.contains("BEGIN OPENSSH PRIVATE KEY")
        || content.contains("BEGIN ENCRYPTED PRIVATE KEY")
        || content.contains("BEGIN PGP PRIVATE KEY BLOCK")
        || content.contains("BEGIN RSA PRIVATE KEY")
        || content.contains("BEGIN EC PRIVATE KEY")
        || content.contains("BEGIN DSA PRIVATE KEY")
        || content.contains("BEGIN SSH2 ENCRYPTED PRIVATE KEY")
        || content.contains("PuTTY-User-Key-File-")
        || content.contains("AGE-SECRET-KEY-")
    {
        return Err(PrivacyRejection::PrivateKey);
    }
    if content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(is_aws_access_key)
    {
        return Err(PrivacyRejection::AwsAccessKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("ghp_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("gho_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("ghu_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::GitHubPat);
    }
    if content
        .split_whitespace()
        .any(|token| token.starts_with("github_pat_") && token.len() >= 40)
    {
        return Err(PrivacyRejection::GitHubFineGrainedPat);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("glpat-") && token.len() >= 20)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("hf_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("lin_api_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("npm_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("tskey-auth-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("tskey-api-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content.split_whitespace().any(is_jwt) {
        return Err(PrivacyRejection::Jwt);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk-ant-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::AnthropicKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk_live_") && token.len() >= 30)
    {
        return Err(PrivacyRejection::StripeLiveKey);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| {
            (token.starts_with("xoxb-") || token.starts_with("xoxp-") || token.starts_with("xapp-"))
                && token.len() >= 20
        })
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    if content
        .split(|ch: char| ch.is_whitespace() || ch == '=')
        .any(|token| token.starts_with("sk-") && token.len() >= 30)
    {
        return Err(PrivacyRejection::OpenAiKey);
    }
    if content
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .any(|token| token.len() > 20 && shannon_entropy(token) > 4.5)
    {
        return Err(PrivacyRejection::HighEntropy);
    }
    Ok(())
}

fn shannon_entropy(token: &str) -> f64 {
    let mut counts = [0usize; 256];
    for byte in token.bytes() {
        counts[byte as usize] += 1;
    }

    let len = token.len() as f64;
    counts
        .into_iter()
        .filter(|count| *count > 0)
        .map(|count| {
            let p = count as f64 / len;
            -p * p.log2()
        })
        .sum()
}

fn is_jwt(token: &str) -> bool {
    let mut parts = token.split('.');
    matches!(
        (parts.next(), parts.next(), parts.next(), parts.next()),
        (Some(header), Some(claims), Some(signature), None)
            if header.starts_with("eyJ")
                && header.len() >= 10
                && claims.len() >= 10
                && signature.len() >= 10
    )
}

fn is_aws_access_key(token: &str) -> bool {
    token.len() == 20
        && token.starts_with("AKIA")
        && token
            .chars()
            .all(|ch| ch.is_ascii_uppercase() || ch.is_ascii_digit())
}
