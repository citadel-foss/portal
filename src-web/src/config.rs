//! Server-owned runtime options. Deliberately not reachable from the browser: a deployment
//! supplies these through its launcher, and an API field for any of them would let a session
//! move the data root or widen the access policy.

use std::net::SocketAddr;
use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AccessProfile {
    /// Loopback only, plain HTTP. The owner password is still required: another account on
    /// the same machine can reach 127.0.0.1 too.
    DevelopmentLoopback,
    /// A TLS-terminating proxy in front; the backend port must not be reachable directly.
    TrustedTlsProxy,
    /// A trusted proxy in front that does NOT terminate TLS, so the browser link is plain
    /// HTTP: Umbrel's `app_proxy` on the LAN, or a published container port in development.
    /// Session cookies therefore travel in clear, exactly as on the onion profile, which is
    /// only acceptable because the proxy and the browser share a private network.
    TrustedHttpProxy,
    /// Onion-only, enabled per release after its cookie/origin/bypass tests pass.
    OnlyOnion,
}

#[derive(Debug, Parser)]
#[command(name = "portal-web", about = "Self-hosted Portal")]
pub struct Config {
    /// Address to bind. A container wrapper selects its own internal interface explicitly.
    #[arg(long, default_value = "127.0.0.1:3000", env = "PORTAL_BIND")]
    pub bind: SocketAddr,

    /// Built frontend to serve. Required for a production start; omitted in development,
    /// where Vite serves the UI and proxies the API here. Only this directory is served;
    /// API errors never fall back to `index.html`.
    #[arg(long, env = "PORTAL_ASSETS_DIR")]
    pub assets_dir: Option<PathBuf>,

    /// Wallet data root. Defaults to the crate's own location under the process home, and a
    /// mismatch with that default is a startup error rather than a silent second tree.
    #[arg(long, env = "PORTAL_DATA_ROOT")]
    pub data_root: Option<PathBuf>,

    /// Exactly the origin browsers use. Required outside development: Host, Origin and cookie
    /// policy are all derived from it.
    #[arg(long, env = "PORTAL_PUBLIC_ORIGIN")]
    pub public_origin: Option<String>,

    /// One normalized prefix shared by assets, API routes, cookies and proxy routing.
    #[arg(long, default_value = "/", env = "PORTAL_BASE_PATH")]
    pub base_path: String,

    #[arg(long, value_enum, default_value = "development-loopback", env = "PORTAL_ACCESS_PROFILE")]
    pub access_profile: AccessProfile,

    /// Exact proxy peers permitted to supply forwarded client addresses.
    #[arg(long, env = "PORTAL_TRUSTED_PROXY")]
    pub trusted_proxy: Vec<String>,

    /// Private, read-only file holding a provisioned owner credential. Never echoed, and
    /// never passed as a password argument where a process list would show it.
    #[arg(long, env = "PORTAL_OWNER_CREDENTIAL_FILE")]
    pub owner_credential_file: Option<PathBuf>,


    /// Probe the liveness endpoint of an already-running instance and exit 0 or 1, then
    /// stop. Exists so the container can health-check itself without shipping curl into a
    /// slim runtime image.
    #[arg(long)]
    pub healthcheck: bool,

    /// Must be shorter than the supervisor's verified stop grace.
    #[arg(long, default_value_t = 110, env = "PORTAL_SHUTDOWN_TIMEOUT_SECS")]
    pub shutdown_timeout_secs: u64,
}

impl Config {
    /// Rejects combinations that would serve wallet data over an unprotected transport. There
    /// is deliberately no override: a target that cannot provide a tested protected path is
    /// not ready for release, and a flag to skip this would become the way every deployment
    /// runs.
    pub fn validate(&self) -> Result<(), String> {
        if !self.base_path.starts_with('/') {
            return Err("--base-path must start with '/'".into());
        }
        if self.assets_dir.is_none() && self.access_profile != AccessProfile::DevelopmentLoopback {
            return Err("--assets-dir is required outside development".into());
        }
        match self.access_profile {
            AccessProfile::DevelopmentLoopback => {
                if !self.bind.ip().is_loopback() {
                    return Err(
                        "development-loopback may bind only a loopback address; a reachable \
                         bind needs trusted-tls-proxy or onion-only"
                            .into(),
                    );
                }
            }
            AccessProfile::TrustedTlsProxy | AccessProfile::OnlyOnion | AccessProfile::TrustedHttpProxy => {
                let origin = self
                    .public_origin
                    .as_deref()
                    .ok_or("--public-origin is required outside development")?;
                // The proxy terminates TLS, so the backend itself binds plain HTTP; what must
                // be HTTPS is the origin the browser actually uses.
                if self.access_profile == AccessProfile::TrustedTlsProxy
                    && !origin.starts_with("https://")
                {
                    return Err("trusted-tls-proxy requires an https --public-origin".into());
                }
                if self.access_profile == AccessProfile::OnlyOnion && !origin.contains(".onion") {
                    return Err("onion-only requires an .onion --public-origin".into());
                }
                // Refused rather than silently downgraded: an https origin here would hand the
                // browser a cookie without `Secure`, which is the one mistake this profile
                // must not make quietly.
                if self.access_profile == AccessProfile::TrustedHttpProxy
                    && !origin.starts_with("http://")
                {
                    return Err(
                        "trusted-http-proxy requires an http:// --public-origin; an https \
                         origin belongs to trusted-tls-proxy"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Cookies are marked `Secure` wherever the browser is actually on HTTPS. An onion origin
    /// is already authenticated and encrypted by Tor, and browsers do not treat plain-HTTP
    /// onion pages as secure contexts for cookie purposes, so it is excluded here.
    pub fn secure_cookies(&self) -> bool {
        matches!(self.access_profile, AccessProfile::TrustedTlsProxy)
    }

    /// The URL to put in front of a person, which is not always what the socket is bound to.
    /// A wildcard bind has no browsable form, and behind a proxy the browser never touches
    /// this port at all \u2014 so a configured origin always wins.
    pub fn browsable_url(&self) -> String {
        let path = self.base_path.trim_end_matches('/');
        if let Some(origin) = &self.public_origin {
            return format!("{}{path}", origin.trim_end_matches('/'));
        }
        let ip = self.bind.ip();
        let host = if ip.is_unspecified() {
            // 0.0.0.0 is not an address anyone can open. Loopback is the one that always
            // works from the machine the server is on, which is who reads this line.
            if ip.is_ipv6() { "[::1]".to_string() } else { "127.0.0.1".to_string() }
        } else if ip.is_ipv6() {
            format!("[{ip}]")
        } else {
            ip.to_string()
        };
        format!("http://{host}:{}{path}", self.bind.port())
    }

    pub fn route(&self, suffix: &str) -> String {
        let base = self.base_path.trim_end_matches('/');
        format!("{base}{suffix}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(profile: AccessProfile, bind: &str, origin: Option<&str>) -> Config {
        Config {
            bind: bind.parse().unwrap(),
            assets_dir: Some(PathBuf::from("dist/web")),
            data_root: None,
            public_origin: origin.map(str::to_string),
            base_path: "/".into(),
            access_profile: profile,
            trusted_proxy: vec![],
            owner_credential_file: None,
            shutdown_timeout_secs: 110,
            healthcheck: false,
        }
    }

    #[test]
    fn development_may_not_bind_a_reachable_address() {
        assert!(config(AccessProfile::DevelopmentLoopback, "127.0.0.1:3000", None)
            .validate()
            .is_ok());
        assert!(config(AccessProfile::DevelopmentLoopback, "0.0.0.0:3000", None)
            .validate()
            .is_err());
    }

    /// A proxy deployment legitimately binds a container interface without local TLS, so the
    /// check is on the browser-facing origin rather than on the bind address.
    #[test]
    fn a_proxy_profile_binds_freely_but_needs_an_https_origin() {
        assert!(
            config(AccessProfile::TrustedTlsProxy, "0.0.0.0:3000", Some("https://portal.example"))
                .validate()
                .is_ok()
        );
        assert!(
            config(AccessProfile::TrustedTlsProxy, "0.0.0.0:3000", Some("http://portal.example"))
                .validate()
                .is_err()
        );
        assert!(config(AccessProfile::TrustedTlsProxy, "0.0.0.0:3000", None)
            .validate()
            .is_err());
    }

    /// The Umbrel `app_proxy` case: reachable bind, plain-HTTP origin, no `Secure` cookie.
    #[test]
    fn a_plain_http_proxy_binds_freely_but_refuses_an_https_origin() {
        let umbrel = config(
            AccessProfile::TrustedHttpProxy,
            "0.0.0.0:3000",
            Some("http://umbrel.local:3000"),
        );
        assert!(umbrel.validate().is_ok());
        assert!(!umbrel.secure_cookies());
        assert!(config(
            AccessProfile::TrustedHttpProxy,
            "0.0.0.0:3000",
            Some("https://portal.example")
        )
        .validate()
        .is_err());
        assert!(config(AccessProfile::TrustedHttpProxy, "0.0.0.0:3000", None)
            .validate()
            .is_err());
    }

    #[test]
    fn onion_only_requires_an_onion_origin() {
        assert!(config(AccessProfile::OnlyOnion, "0.0.0.0:3000", Some("http://abc.onion"))
            .validate()
            .is_ok());
        assert!(
            config(AccessProfile::OnlyOnion, "0.0.0.0:3000", Some("https://portal.example"))
                .validate()
                .is_err()
        );
    }

    /// Production must not start without a UI to serve; development legitimately has none,
    /// because Vite is serving it.
    #[test]
    fn assets_are_required_outside_development() {
        let mut c = config(AccessProfile::TrustedTlsProxy, "0.0.0.0:3000", Some("https://p.example"));
        c.assets_dir = None;
        assert!(c.validate().is_err());
        let mut dev = config(AccessProfile::DevelopmentLoopback, "127.0.0.1:3000", None);
        dev.assets_dir = None;
        assert!(dev.validate().is_ok());
    }

    /// The line a person scrolls back to find, so it has to be openable as printed.
    #[test]
    fn the_browsable_url_is_one_a_browser_can_actually_open() {
        // A wildcard bind is not an address; loopback is.
        assert_eq!(
            config(AccessProfile::DevelopmentLoopback, "127.0.0.1:3000", None).browsable_url(),
            "http://127.0.0.1:3000"
        );
        let mut wildcard = config(AccessProfile::DevelopmentLoopback, "0.0.0.0:3000", None);
        wildcard.public_origin = None;
        assert_eq!(wildcard.browsable_url(), "http://127.0.0.1:3000");

        // Behind a proxy the bind is irrelevant \u2014 the origin is what the browser uses.
        let proxied = config(
            AccessProfile::TrustedTlsProxy,
            "0.0.0.0:3000",
            Some("https://portal.example"),
        );
        assert_eq!(proxied.browsable_url(), "https://portal.example");

        let mut based = config(AccessProfile::DevelopmentLoopback, "127.0.0.1:3000", None);
        based.base_path = "/portal".into();
        assert_eq!(based.browsable_url(), "http://127.0.0.1:3000/portal");
    }

    #[test]
    fn routes_are_joined_under_one_base_path() {
        let mut c = config(AccessProfile::DevelopmentLoopback, "127.0.0.1:3000", None);
        assert_eq!(c.route("/api/v1/session"), "/api/v1/session");
        c.base_path = "/portal".into();
        assert_eq!(c.route("/api/v1/session"), "/portal/api/v1/session");
    }
}
