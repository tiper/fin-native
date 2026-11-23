//! The launch string the host hands `plx_host_start`: what the application boots with, as typed values.
//!
//! The keys are the header's (`include/plx_host.h`, "LAUNCH JSON"): the payload and data directories, the
//! optional Plex server, locale, Tizen version, UDP log target and dev-boot values.
//! Unknown keys are ignored (a newer host may send more); a string that is not JSON, a required key that is
//! missing or of the wrong type, and a value out of range are a refusal ([`LaunchError`]), which the
//! hosted contract reports as `PLX_HOST_E_LAUNCH` and `on_stopped(1, 1)`.
//!
//! **The text carries the dev token and is never logged, quoted or put in an error**: [`LaunchError`] names
//! the key and the rule it broke, [`Dev`]'s `Debug` redacts the token. Pure: no IO, no process state.
use std::net::SocketAddrV4;
use std::path::PathBuf;

/// The launch values, validated.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Launch {
    /// The package's read-only payload directory (`res/`).
    pub payload_dir: PathBuf,
    /// The app's writable data directory.
    pub state_dir: PathBuf,
    /// A fixed Plex server for dev boots. With no server, the app starts at sign-in.
    pub pms: Option<(std::ffi::CString, u16)>,
    /// A BCP-47 tag; `en-US` when the host sent none.
    pub locale: String,
    /// The Tizen version as the host read it (`10.0`); `unknown` when it sent none.
    pub platform_version: String,
    /// Where the log's datagrams go; `None` is the channel off.
    pub log_udp: Option<SocketAddrV4>,
    pub dev: Dev,
}

/// The dev-boot values (`devtriggers` builds only act on them): a placeholder identity for the mock server
/// and the mock's origin.
#[derive(Clone, PartialEq, Eq, Default)]
pub struct Dev {
    pub token: Option<String>,
    pub pms_origin: Option<String>,
}

impl std::fmt::Debug for Dev {
    /// The token never prints, in a log, a panic message or a failed assertion.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Dev")
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("pms_origin", &self.pms_origin)
            .finish()
    }
}

/// Why a launch string was refused: the key, and the rule. Never any of the text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchError(pub String);

impl std::fmt::Display for LaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "launch refused: {}", self.0)
    }
}

fn refuse<T>(why: impl Into<String>) -> Result<T, LaunchError> {
    Err(LaunchError(why.into()))
}

/// A string value: absent is `None`, any other type is a refusal, NUL is a refusal (the application passes
/// these on as C strings).
fn string_of(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<Option<String>, LaunchError> {
    match obj.get(key) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) if s.contains('\0') => refuse(format!("`{key}` has a NUL")),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => refuse(format!("`{key}` is not a string")),
    }
}

fn required(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<String, LaunchError> {
    match string_of(obj, key)? {
        Some(s) if !s.trim().is_empty() => Ok(s),
        _ => refuse(format!("`{key}` is missing or empty")),
    }
}

fn absolute(obj: &serde_json::Map<String, serde_json::Value>, key: &str) -> Result<PathBuf, LaunchError> {
    let path = PathBuf::from(required(obj, key)?);
    if path.is_absolute() {
        Ok(path)
    } else {
        refuse(format!("`{key}` is not an absolute path"))
    }
}

impl Launch {
    /// Parse and validate `text`.
    pub fn parse(text: &str) -> Result<Launch, LaunchError> {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return refuse("not JSON");
        };
        let Some(obj) = value.as_object() else {
            return refuse("not a JSON object");
        };
        let payload_dir = absolute(obj, "payload_dir")?;
        let state_dir = absolute(obj, "state_dir")?;
        let pms = match (obj.contains_key("pms_host"), obj.get("pms_port")) {
            (false, None) => None,
            (_, port) => {
                let host = required(obj, "pms_host")?;
                let Some(port @ 1..=65535) = port.and_then(serde_json::Value::as_u64) else {
                    return refuse("`pms_port` is missing or not an integer in 1..=65535");
                };
                let host = std::ffi::CString::new(host)
                    .map_err(|_| LaunchError("`pms_host` has a NUL".to_string()))?;
                Some((host, port as u16))
            }
        };
        let locale = string_of(obj, "locale")?.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "en-US".to_string());
        let platform_version =
            string_of(obj, "platform_version")?.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| "unknown".to_string());
        let log_udp = match string_of(obj, "log_udp")?.map(|s| s.trim().to_string()).filter(|s| !s.is_empty()) {
            None => None,
            Some(target) => match target.parse::<SocketAddrV4>() {
                Ok(addr) => Some(addr),
                Err(_) => return refuse("`log_udp` is not an IPv4 literal `a.b.c.d:port`"),
            },
        };
        let dev = match obj.get("dev") {
            None | Some(serde_json::Value::Null) => Dev::default(),
            Some(serde_json::Value::Object(dev)) => Dev {
                token: string_of(dev, "token").map_err(|e| LaunchError(format!("dev {}", e.0)))?.filter(|s| !s.is_empty()),
                pms_origin: string_of(dev, "pms_origin")
                    .map_err(|e| LaunchError(format!("dev {}", e.0)))?
                    .filter(|s| !s.is_empty()),
            },
            Some(_) => return refuse("`dev` is not an object"),
        };
        Ok(Launch { payload_dir, state_dir, pms, locale, platform_version, log_udp, dev })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: &str = r#"{"payload_dir":"/res","state_dir":"/data","pms_host":"10.0.2.2","pms_port":32499}"#;

    fn with(extra: &str) -> String {
        format!("{}, {extra}}}", MIN.trim_end_matches('}'))
    }

    fn refused(text: &str) -> String {
        Launch::parse(text).expect_err("refused").0
    }

    #[test]
    fn a_package_without_a_dev_boot_can_start_signed_out() {
        let launch = Launch::parse(r#"{"payload_dir":"/res","state_dir":"/data"}"#)
            .expect("a package without a fixed Plex server can launch");
        assert_eq!(launch.pms, None);
    }

    #[test]
    fn the_minimal_string_parses_with_the_defaults() {
        let l = Launch::parse(MIN).expect("parses");
        assert_eq!(l.payload_dir, PathBuf::from("/res"));
        assert_eq!(l.state_dir, PathBuf::from("/data"));
        assert_eq!(l.pms, Some((c"10.0.2.2".to_owned(), 32499)));
        assert_eq!((l.locale.as_str(), l.platform_version.as_str()), ("en-US", "unknown"));
        assert_eq!(l.log_udp, None, "the log channel is off unless named");
        assert_eq!(l.dev, Dev::default());
    }

    #[test]
    fn every_key_is_read_and_an_unknown_key_is_ignored() {
        let l = Launch::parse(&with(
            r#""locale":"be-BY","platform_version":"10.0","log_udp":" 10.0.2.2:5514 ","future_key":{"a":1},
               "dev":{"token":"placeholder","pms_origin":"http://10.0.2.2:32499","other":1}"#,
        ))
        .expect("parses");
        assert_eq!((l.locale.as_str(), l.platform_version.as_str()), ("be-BY", "10.0"));
        assert_eq!(l.log_udp, Some("10.0.2.2:5514".parse().unwrap()));
        assert_eq!(l.dev.token.as_deref(), Some("placeholder"));
        assert_eq!(l.dev.pms_origin.as_deref(), Some("http://10.0.2.2:32499"));
    }

    /// A malformed string is a refusal, whatever is malformed, and the reason names the key and never the text.
    #[test]
    fn a_malformed_string_is_refused_without_quoting_it() {
        assert_eq!(refused("{not json"), "not JSON");
        assert_eq!(refused("[1,2]"), "not a JSON object");
        assert_eq!(refused(""), "not JSON");
        for (text, key) in [
            (r#"{"state_dir":"/d","pms_host":"h","pms_port":1}"#, "payload_dir"),
            (r#"{"payload_dir":"/r","pms_host":"h","pms_port":1}"#, "state_dir"),
            (r#"{"payload_dir":"/r","state_dir":"/d","pms_port":1}"#, "pms_host"),
            (r#"{"payload_dir":"/r","state_dir":"/d","pms_host":"h"}"#, "pms_port"),
            (r#"{"payload_dir":7,"state_dir":"/d","pms_host":"h","pms_port":1}"#, "payload_dir"),
            (r#"{"payload_dir":"rel/res","state_dir":"/d","pms_host":"h","pms_port":1}"#, "payload_dir"),
            (r#"{"payload_dir":"/r","state_dir":"","pms_host":"h","pms_port":1}"#, "state_dir"),
            (r#"{"payload_dir":"/r","state_dir":"/d","pms_host":"a\u0000b","pms_port":1}"#, "pms_host"),
        ] {
            let why = refused(text);
            assert!(why.contains(&format!("`{key}`")), "{text} -> {why}");
        }
        for port in ["0", "65536", "-1", "\"32400\"", "32400.5", "null"] {
            let text = format!(r#"{{"payload_dir":"/r","state_dir":"/d","pms_host":"h","pms_port":{port}}}"#);
            assert!(refused(&text).contains("`pms_port`"), "port {port}");
        }
        let planted = refused(&with(r#""dev":{"token":["SECRET-TOKEN"]}"#));
        assert!(planted.contains("token"), "{planted}");
        assert!(!planted.contains("SECRET"), "the refusal never quotes the text: {planted}");
    }

    /// The log target is an IPv4 literal with a port, or nothing: a host name, an IPv6 literal and a bare
    /// address are refused (the channel is off unless it can be named without a lookup), and an empty
    /// string is "off", not an error.
    #[test]
    fn the_log_target_is_an_ipv4_literal_or_absent() {
        for bad in ["example.com:5514", "[::1]:5514", "10.0.2.2", "10.0.2.2:0x10", "10.0.2.256:1"] {
            assert!(refused(&with(&format!(r#""log_udp":"{bad}""#))).contains("`log_udp`"), "{bad}");
        }
        assert!(refused(&with(r#""log_udp":5514"#)).contains("`log_udp`"), "not a string");
        for off in [r#""log_udp":"""#, r#""log_udp":"  ""#, r#""log_udp":null"#] {
            assert_eq!(Launch::parse(&with(off)).unwrap().log_udp, None, "{off}");
        }
    }

    #[test]
    fn the_dev_values_have_their_types_and_the_token_never_prints() {
        assert!(refused(&with(r#""dev":"on""#)).contains("`dev`"));
        assert!(refused(&with(r#""dev":{"pms_origin":5}"#)).contains("pms_origin"));
        let l = Launch::parse(&with(r#""dev":{"token":"SECRET-TOKEN","pms_origin":""}"#)).unwrap();
        assert_eq!(l.dev.token.as_deref(), Some("SECRET-TOKEN"));
        assert_eq!(l.dev.pms_origin, None, "an empty origin is absent");
        let shown = format!("{l:?} {:?}", l.dev);
        assert!(!shown.contains("SECRET"), "{shown}");
        assert!(shown.contains("<redacted>"), "{shown}");
    }
}
