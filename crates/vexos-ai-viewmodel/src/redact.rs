//! Redaction of collected log text before anyone sees it. Only obvious
//! secrets are caught: private key blocks, authorization headers and bearer
//! tokens, `password=`-style values, well-known token prefixes, passwords in
//! URLs and long random-looking strings. Home directory names become
//! `[user]`. The user still reviews the whole draft before it is sent.

/// What a secret is replaced with.
pub const MARK: &str = "[redacted]";
const KEY_MARK: &str = "[redacted private key]";
const USER_MARK: &str = "[user]";

/// What was replaced.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Redactions {
    /// Values that looked like secrets.
    pub secrets: usize,
    /// Home directory names.
    pub homes: usize,
}

pub fn redact(text: &str) -> (String, Redactions) {
    let mut r = Redactions::default();
    let lines: Vec<String> = key_blocks(text, &mut r.secrets)
        .iter()
        .map(|line| {
            let line = url_passwords(line, &mut r.secrets);
            let line = key_values(&line, &mut r.secrets);
            let line = scheme_tokens(&line, &mut r.secrets);
            let line = prefixed_tokens(&line, &mut r.secrets);
            let line = random_strings(&line, &mut r.secrets);
            home_paths(&line, &mut r.homes)
        })
        .collect();
    (lines.join("\n"), r)
}

fn is_key_begin(line: &str) -> bool {
    line.contains("-----BEGIN ") && line.contains("PRIVATE KEY")
}

fn is_key_end(line: &str) -> bool {
    line.contains("-----END ") && line.contains("PRIVATE KEY")
}

/// A line that is (or ends in) a run of base64, as in a key body.
fn looks_base64(line: &str) -> bool {
    line.split_whitespace().next_back().is_some_and(|word| {
        word.len() >= 16
            && word
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"+/=".contains(&b))
    })
}

/// Replaces private key blocks. An unterminated block hides everything after
/// it; an END without its BEGIN (the excerpt started inside the block) hides
/// the base64 lines before it.
fn key_blocks(text: &str, count: &mut usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut inside = false;
    for line in text.split('\n') {
        if inside {
            inside = !is_key_end(line);
            continue;
        }
        if let Some(begin) = line.find("-----BEGIN ").filter(|_| is_key_begin(line)) {
            *count += 1;
            match inline_block_end(line, begin) {
                Some(end) => out.push(format!("{}{KEY_MARK}{}", &line[..begin], &line[end..])),
                None => {
                    inside = true;
                    out.push(format!("{}{KEY_MARK}", &line[..begin]));
                }
            }
        } else if is_key_end(line) {
            while out.last().is_some_and(|l| looks_base64(l)) {
                out.pop();
            }
            *count += 1;
            out.push(KEY_MARK.to_owned());
        } else {
            out.push(line.to_owned());
        }
    }
    out
}

/// Where a key block that ends on its BEGIN line stops.
fn inline_block_end(line: &str, begin: usize) -> Option<usize> {
    let end = begin + line[begin..].find("-----END ")?;
    let key = end + line[end..].find("PRIVATE KEY")?;
    let close = key + line[key..].find("-----")?;
    Some(close + "-----".len())
}

/// `scheme://user:password@host` keeps the user and hides the password.
fn url_passwords(line: &str, count: &mut usize) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("://") {
        let (head, tail) = rest.split_at(i + 3);
        out.push_str(head);
        let end = tail
            .find(|c: char| c.is_whitespace() || "/?#\"'<>".contains(c))
            .unwrap_or(tail.len());
        let authority = &tail[..end];
        rest = tail;
        if let Some(at) = authority.rfind('@')
            && let Some(colon) = authority[..at].find(':')
            && colon + 1 < at
        {
            *count += 1;
            out.push_str(&authority[..=colon]);
            out.push_str(MARK);
            out.push_str(&authority[at..]);
            rest = &tail[end..];
        }
    }
    out.push_str(rest);
    out
}

/// Parts of a key name that make its value a secret.
const KEYS: &[&str] = &[
    "password",
    "passwd",
    "passphrase",
    "secret",
    "token",
    "apikey",
    "api_key",
    "api-key",
    "accesskey",
    "access_key",
    "access-key",
    "private_key",
    "private-key",
    "authorization",
    "cookie",
    "credential",
];

/// Words in an `Authorization` value that come before the credential.
const SCHEMES: &[&str] = &["bearer", "basic", "token", "digest", "negotiate"];

fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_value_end(b: u8) -> bool {
    b.is_ascii_whitespace() || b",;&)}]\"'".contains(&b)
}

/// `key=value`, `key: value`, `"key": "value"` and `--key=value` for a
/// secret-sounding key: the value is hidden.
fn key_values(line: &str, count: &mut usize) -> String {
    let lower = line.to_ascii_lowercase();
    let b = line.as_bytes();
    let n = b.len();
    let mut out = String::new();
    let mut copied = 0;
    let mut i = 0;
    while i < n {
        if !is_ident(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < n && is_ident(b[i]) {
            i += 1;
        }
        let ident = &lower[start..i];
        if !KEYS.iter().any(|k| ident.contains(k)) {
            continue;
        }
        let mut j = i;
        if j < n && (b[j] == b'"' || b[j] == b'\'') {
            j += 1;
        }
        while j < n && b[j] == b' ' {
            j += 1;
        }
        if j >= n || !(b[j] == b'=' || b[j] == b':') {
            continue;
        }
        // `token::path` is code, not a value.
        if b[j] == b':' && j + 1 < n && b[j + 1] == b':' {
            continue;
        }
        j += 1;
        while j < n && b[j] == b' ' {
            j += 1;
        }
        let (from, to) = if j < n && (b[j] == b'"' || b[j] == b'\'') {
            let quote = b[j];
            let to = (j + 1..n).find(|&k| b[k] == quote).unwrap_or(n);
            (j + 1, to)
        } else {
            let mut to = (j..n).find(|&k| is_value_end(b[k])).unwrap_or(n);
            // `Authorization: Bearer <credential>`: hide the credential too.
            if SCHEMES.contains(&&lower[j..to]) {
                let next = (to..n).find(|&k| b[k] != b' ').unwrap_or(n);
                to = (next..n).find(|&k| is_value_end(b[k])).unwrap_or(n);
            }
            (j, to)
        };
        i = to.max(i);
        let value = &lower[from..to];
        let harmless = value.is_empty()
            || value == MARK
            || ["true", "false", "null", "none", "yes", "no"].contains(&value)
            || (value.len() < 8 && value.bytes().all(|c| c.is_ascii_digit()));
        if harmless {
            continue;
        }
        *count += 1;
        out.push_str(&line[copied..from]);
        out.push_str(MARK);
        copied = to;
    }
    out.push_str(&line[copied..]);
    out
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"._~+/=-".contains(&b)
}

/// `Bearer <token>` and `Basic <credentials>` anywhere in a line.
fn scheme_tokens(line: &str, count: &mut usize) -> String {
    let lower = line.to_ascii_lowercase();
    let b = line.as_bytes();
    let mut out = String::new();
    let mut copied = 0;
    for scheme in ["bearer ", "basic "] {
        let mut from = 0;
        while let Some(pos) = lower[from..].find(scheme).map(|p| p + from) {
            from = pos + scheme.len();
            if (pos > 0 && b[pos - 1].is_ascii_alphanumeric()) || pos < copied {
                continue;
            }
            let start = (from..b.len()).find(|&k| b[k] != b' ').unwrap_or(b.len());
            let end = (start..b.len())
                .find(|&k| !is_token_char(b[k]))
                .unwrap_or(b.len());
            let token = &line[start..end];
            let digit = token.bytes().any(|c| c.is_ascii_digit());
            let mixed = token.bytes().any(|c| c.is_ascii_uppercase())
                && token.bytes().any(|c| c.is_ascii_lowercase());
            if token.len() >= 8 && (digit || mixed) {
                *count += 1;
                out.push_str(&line[copied..start]);
                out.push_str(MARK);
                copied = end;
                from = end;
            }
        }
    }
    out.push_str(&line[copied..]);
    out
}

/// Token formats that announce themselves.
const PREFIXES: &[&str] = &[
    "sk-ant-",
    "sk-",
    "github_pat_",
    "ghp_",
    "gho_",
    "ghu_",
    "ghs_",
    "ghr_",
    "glpat-",
    "xoxb-",
    "xoxp-",
    "xoxa-",
    "xoxs-",
    "npm_",
    "AIza",
];

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn is_prefixed_token(word: &str) -> bool {
    if word.len() == 20
        && word.starts_with("AKIA")
        && word
            .bytes()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
    {
        return true;
    }
    PREFIXES.iter().any(|p| {
        word.strip_prefix(p).is_some_and(|rest| {
            rest.len() >= 16
                && rest
                    .bytes()
                    .any(|c| c.is_ascii_digit() || c.is_ascii_uppercase())
        })
    })
}

fn prefixed_tokens(line: &str, count: &mut usize) -> String {
    replace_runs(line, is_word_char, |word| {
        let hit = is_prefixed_token(word);
        *count += usize::from(hit);
        hit
    })
}

/// Shannon entropy, bits per character.
fn entropy(s: &str) -> f64 {
    let mut freq = [0usize; 256];
    for b in s.bytes() {
        freq[usize::from(b)] += 1;
    }
    let n = s.len() as f64;
    freq.iter()
        .filter(|&&c| c > 0)
        .map(|&c| {
            let p = c as f64 / n;
            -p * p.log2()
        })
        .sum()
}

/// Long, mixed-case, with digits, and random-looking. Hex digests and nix
/// store hashes have no upper case, so they never match; SRI hashes (as in a
/// hash mismatch) are kept on purpose.
fn looks_random(s: &str) -> bool {
    s.len() >= 32
        && !["sha1-", "sha256-", "sha384-", "sha512-"]
            .iter()
            .any(|p| s.starts_with(p))
        && s.bytes().any(|c| c.is_ascii_uppercase())
        && s.bytes().any(|c| c.is_ascii_lowercase())
        && s.bytes().any(|c| c.is_ascii_digit())
        && entropy(s) >= 4.0
}

fn is_base64_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"+/=_-".contains(&b)
}

/// A random-looking run is hidden whole; in a path (a run that starts with
/// `/`) only the random-looking parts between slashes are.
fn random_strings(line: &str, count: &mut usize) -> String {
    let mut out = String::new();
    let mut last = 0;
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !is_base64_char(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_base64_char(b[i]) {
            i += 1;
        }
        let run = &line[start..i];
        let replaced = if run.starts_with('/') {
            let parts: Vec<&str> = run.split('/').collect();
            if !parts.iter().any(|p| looks_random(p)) {
                continue;
            }
            parts
                .iter()
                .map(|p| {
                    if looks_random(p) {
                        *count += 1;
                        MARK
                    } else {
                        p
                    }
                })
                .collect::<Vec<_>>()
                .join("/")
        } else if looks_random(run) {
            *count += 1;
            MARK.to_owned()
        } else {
            continue;
        };
        out.push_str(&line[last..start]);
        out.push_str(&replaced);
        last = i;
    }
    out.push_str(&line[last..]);
    out
}

/// `/home/<name>` becomes `/home/[user]`.
fn home_paths(line: &str, count: &mut usize) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find("/home/") {
        let (head, tail) = rest.split_at(i + "/home/".len());
        out.push_str(head);
        let end = tail
            .find(|c: char| !(c.is_alphanumeric() || "._-".contains(c)))
            .unwrap_or(tail.len());
        if end > 0 {
            *count += 1;
            out.push_str(USER_MARK);
        }
        rest = &tail[end..];
    }
    out.push_str(rest);
    out
}

/// Replaces each maximal run of `in_run` bytes for which `hit` says so.
fn replace_runs(line: &str, in_run: fn(u8) -> bool, mut hit: impl FnMut(&str) -> bool) -> String {
    let b = line.as_bytes();
    let mut out = String::new();
    let mut last = 0;
    let mut i = 0;
    while i < b.len() {
        if !in_run(b[i]) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && in_run(b[i]) {
            i += 1;
        }
        if hit(&line[start..i]) {
            out.push_str(&line[last..start]);
            out.push_str(MARK);
            last = i;
        }
    }
    out.push_str(&line[last..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn red(text: &str) -> String {
        redact(text).0
    }

    // Fake values, shaped like the real thing. Each is split in the source
    // so secret scanners (e.g. GitHub push protection) don't flag the repo.
    const ANTHROPIC: &str = concat!("sk-", "ant-api03-", "Zq8xV2mN4pL7rT1wY6uB9cD3fG5hJ0kA");
    const GITHUB: &str = concat!("ghp_", "R4nd0mT0k3nV4lu3F0rT3st1ngXyZ12");
    const GITLAB: &str = concat!("glpat-", "Ab3dEf6hIj9kLm2nOp5q");
    const AWS_ID: &str = concat!("AKIA", "IOSFODNN7EXAMPLE");
    const AWS_SECRET: &str = concat!("wJalrXUtnFEMI/", "K7MDENG/bPxRfiCYEXAMPLEKEY");
    const JWT: &str = concat!(
        "eyJhbGciOiJIUzI1NiJ9",
        ".",
        "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
        ".",
        "dBjftJeZ4CVPmB92K27uhbUJU1p1r"
    );

    /// Key block markers, built so the source holds no whole marker line.
    fn begin(kind: &str) -> String {
        format!("-----BEGIN {kind}PRIVATE KEY-----")
    }

    fn end(kind: &str) -> String {
        format!("-----END {kind}PRIVATE KEY-----")
    }

    #[test]
    fn api_keys_and_token_prefixes() {
        for secret in [ANTHROPIC, GITHUB, AWS_ID, GITLAB] {
            let out = red(&format!("export KEY {secret} done"));
            assert!(!out.contains(secret), "{out}");
            assert!(out.contains(MARK) && out.ends_with(" done"), "{out}");
        }
        assert_eq!(red("disk-by-uuid sk-short"), "disk-by-uuid sk-short");
    }

    #[test]
    fn authorization_headers_and_bearer_tokens() {
        let out = red("> Authorization: Bearer abc123DEF456ghi789");
        assert_eq!(out, format!("> Authorization: {MARK}"));
        let out = red(&format!("curl -H 'Authorization: token {GITHUB}' x"));
        assert!(!out.contains("ghp_"), "{out}");
        let out = red(&format!("got bearer {JWT} from server"));
        assert!(!out.contains("eyJ"), "{out}");
        assert!(out.ends_with(" from server"), "{out}");
        let out = red("Basic dXNlcjpwYXNzd29yZDEyMw== sent");
        assert!(!out.contains("dXNlcjpw"), "{out}");
        // English, not credentials.
        assert_eq!(red("a basic configuration"), "a basic configuration");
    }

    #[test]
    fn password_style_values() {
        assert_eq!(red("password=hunter2"), format!("password={MARK}"));
        assert_eq!(
            red("DB_PASSWORD: \"s3cr3t pass\" ok"),
            format!("DB_PASSWORD: \"{MARK}\" ok")
        );
        assert_eq!(
            red("--api-key=abcdef1234 run"),
            format!("--api-key={MARK} run")
        );
        assert_eq!(
            red(r#"{"client_secret":"xyz789","n":1}"#),
            format!(r#"{{"client_secret":"{MARK}","n":1}}"#)
        );
        assert_eq!(
            red("?access_token=abc&x=1"),
            format!("?access_token={MARK}&x=1")
        );
        // Not values.
        assert_eq!(
            red("[sudo] password for alice:"),
            "[sudo] password for alice:"
        );
        assert_eq!(red("tokens: 1234"), "tokens: 1234");
        assert_eq!(red("use token::Kind;"), "use token::Kind;");
        assert_eq!(red("secret: true"), "secret: true");
    }

    #[test]
    fn passwords_in_urls() {
        assert_eq!(
            red("fetching https://bob:pa55word@git.example.com/repo.git"),
            format!("fetching https://bob:{MARK}@git.example.com/repo.git")
        );
        assert_eq!(
            red("postgres://u:p%40ss@db:5432/x and ftp://anon@host/"),
            format!("postgres://u:{MARK}@db:5432/x and ftp://anon@host/")
        );
        assert_eq!(
            red("see https://nixos.org/manual"),
            "see https://nixos.org/manual"
        );
    }

    #[test]
    fn private_key_blocks() {
        let text = format!(
            "before\n{}\nb3BlbnNzaC1rZXktdjEAAAAABG5vbmU\nAAAAAAAAAAEAAAAzAAAAC3NzaC1lZDI1\n{}\nafter",
            begin("OPENSSH "),
            end("OPENSSH ")
        );
        assert_eq!(red(&text), format!("before\n{KEY_MARK}\nafter"));
        // On one line, as in a JSON value.
        let out = red(&format!(
            r#"{{"key":"{}\nMIIEv\n{}\n","x":1}}"#,
            begin(""),
            end("")
        ));
        assert_eq!(out, format!(r#"{{"key":"{KEY_MARK}\n","x":1}}"#));
        // Cut off: nothing after BEGIN survives.
        let out = red(&format!("ok\n{}\nMIIEowIBAAKCAQEA7\nmore", begin("RSA ")));
        assert_eq!(out, format!("ok\n{KEY_MARK}"));
        // The excerpt starts inside the block.
        let out = red(&format!(
            "unrelated line\nMIIEowIBAAKCAQEAxyz12345\nQ2hlY2sgdGhpcyBvdXQ=\n{}\ntail",
            end("RSA ")
        ));
        assert_eq!(out, format!("unrelated line\n{KEY_MARK}\ntail"));
    }

    #[test]
    fn random_looking_strings() {
        let out = red(&format!("aws_secret {AWS_SECRET} end"));
        assert!(!out.contains("wJalr"), "{out}");
        let out = red("id Xk9fQ2LmP7vR3sT8wZ1yB4nC6dE0gH5jK2 x");
        assert_eq!(out, format!("id {MARK} x"));
        let out = red("GET /api/v1/Xk9fQ2LmP7vR3sT8wZ1yB4nC6dE0gH5jK2/status");
        assert_eq!(out, format!("GET /api/v1/{MARK}/status"));
        // What a rebuild failure needs stays.
        for keep in [
            "/nix/store/1b9p07z77phvv2hf6gm9f1d71mxn0l3a-python3.12-PyYAML-6.0.drv",
            "specified: sha256-AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "got:    sha256-Xk9fQ2LmP7vR3sT8wZ1yB4nC6dE0gH5jK2a9bc8Dd0E=",
            "commit 3f786850e387550fdab836ed7e6dc881de23001b",
            "error: attribute 'enableSomethingVeryLongInCamelCaseOption' missing",
        ] {
            assert_eq!(red(keep), keep);
        }
    }

    #[test]
    fn home_directory_names() {
        let (out, r) = redact("open /home/alice/.config/x and /home/bob");
        assert_eq!(out, "open /home/[user]/.config/x and /home/[user]");
        assert_eq!(
            r,
            Redactions {
                secrets: 0,
                homes: 2
            }
        );
        assert_eq!(red("/home/"), "/home/");
    }

    #[test]
    fn counts_and_plain_text() {
        let (out, r) = redact("nothing secret here\nsecond line");
        assert_eq!(out, "nothing secret here\nsecond line");
        assert_eq!(r, Redactions::default());
        let (_, r) = redact(&format!("password=x1\n{GITHUB}"));
        assert_eq!(r.secrets, 2);
    }

    #[test]
    fn non_ascii_text_is_not_split() {
        let text = "Ünïcødé password=pässwörd ✓ /home/jörg";
        let (out, _) = redact(text);
        assert_eq!(out, format!("Ünïcødé password={MARK} ✓ /home/[user]"));
    }
}
