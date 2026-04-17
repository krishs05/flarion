use std::collections::HashMap;

use regex::Regex;

use crate::api::types::ChatCompletionRequest;
use crate::config::Matchers;

/// Request shape extracted from an incoming ChatCompletionRequest + its HTTP headers.
#[allow(dead_code)]
pub struct MatchContext<'a> {
    pub stream: bool,
    pub prompt_tokens: u32,
    pub message_count: u32,
    pub has_system_prompt: bool,
    pub last_user_content: Option<&'a str>,
    pub headers: HashMap<String, String>,
}

impl<'a> MatchContext<'a> {
    #[allow(dead_code)]
    pub fn from_request(
        request: &'a ChatCompletionRequest,
        headers: HashMap<String, String>,
    ) -> Self {
        let prompt_tokens = estimate_prompt_tokens(request);
        let has_system_prompt = request.messages.iter().any(|m| m.role == "system");
        let last_user_content = request
            .messages
            .iter()
            .rev()
            .find(|m| m.role == "user")
            .map(|m| m.content.as_str());
        Self {
            stream: request.stream,
            prompt_tokens,
            message_count: request.messages.len() as u32,
            has_system_prompt,
            last_user_content,
            headers,
        }
    }
}

/// Approximate prompt size by character count / 4.
#[allow(dead_code)]
pub fn estimate_prompt_tokens(request: &ChatCompletionRequest) -> u32 {
    let chars: usize = request
        .messages
        .iter()
        .map(|m| m.content.chars().count())
        .sum();
    (chars / 4) as u32
}

/// A precompiled view of `Matchers` with `content_regex: Option<Regex>`.
pub struct CompiledMatchers {
    pub stream: Option<bool>,
    pub prompt_tokens_gte: Option<u32>,
    pub prompt_tokens_lte: Option<u32>,
    pub message_count_gte: Option<u32>,
    pub message_count_lte: Option<u32>,
    pub has_system_prompt: Option<bool>,
    pub content_regex: Option<Regex>,
    pub header_equals: HashMap<String, String>,
}

impl CompiledMatchers {
    pub fn compile(m: &Matchers) -> Result<Self, regex::Error> {
        let content_regex = match &m.content_regex {
            Some(pat) => Some(Regex::new(pat)?),
            None => None,
        };
        Ok(Self {
            stream: m.stream,
            prompt_tokens_gte: m.prompt_tokens_gte,
            prompt_tokens_lte: m.prompt_tokens_lte,
            message_count_gte: m.message_count_gte,
            message_count_lte: m.message_count_lte,
            has_system_prompt: m.has_system_prompt,
            content_regex,
            header_equals: m.header_equals.clone(),
        })
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.stream.is_none()
            && self.prompt_tokens_gte.is_none()
            && self.prompt_tokens_lte.is_none()
            && self.message_count_gte.is_none()
            && self.message_count_lte.is_none()
            && self.has_system_prompt.is_none()
            && self.content_regex.is_none()
            && self.header_equals.is_empty()
    }

    /// True if every set matcher field is satisfied by the context (AND semantics).
    #[allow(dead_code)]
    pub fn matches(&self, ctx: &MatchContext<'_>) -> bool {
        if let Some(expected) = self.stream
            && ctx.stream != expected
        {
            return false;
        }
        if let Some(min) = self.prompt_tokens_gte
            && ctx.prompt_tokens < min
        {
            return false;
        }
        if let Some(max) = self.prompt_tokens_lte
            && ctx.prompt_tokens > max
        {
            return false;
        }
        if let Some(min) = self.message_count_gte
            && ctx.message_count < min
        {
            return false;
        }
        if let Some(max) = self.message_count_lte
            && ctx.message_count > max
        {
            return false;
        }
        if let Some(expected) = self.has_system_prompt
            && ctx.has_system_prompt != expected
        {
            return false;
        }
        if let Some(ref regex) = self.content_regex {
            let hay = ctx.last_user_content.unwrap_or("");
            if !regex.is_match(hay) {
                return false;
            }
        }
        for (k, v) in &self.header_equals {
            let key_lower = k.to_ascii_lowercase();
            let actual = ctx
                .headers
                .iter()
                .find(|(hk, _)| hk.to_ascii_lowercase() == key_lower)
                .map(|(_, hv)| hv.as_str());
            if actual != Some(v.as_str()) {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::types::ChatMessage;

    fn req(messages: Vec<(&str, &str)>, stream: bool) -> ChatCompletionRequest {
        ChatCompletionRequest {
            model: "m".into(),
            messages: messages
                .into_iter()
                .map(|(r, c)| ChatMessage {
                    role: r.into(),
                    content: c.into(),
                })
                .collect(),
            stream,
            temperature: 0.7,
            top_p: 0.9,
            max_tokens: 16,
            stop: Vec::new(),
            seed: None,
        }
    }

    fn ctx<'a>(request: &'a ChatCompletionRequest, headers: &[(&str, &str)]) -> MatchContext<'a> {
        let map: HashMap<String, String> = headers
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        MatchContext::from_request(request, map)
    }

    #[test]
    fn estimate_tokens_roughly_chars_over_four() {
        let r = req(vec![("user", "abcdefgh")], false);
        assert_eq!(estimate_prompt_tokens(&r), 2);
    }

    #[test]
    fn empty_matchers_match_everything() {
        let m = CompiledMatchers::compile(&Matchers::default()).unwrap();
        let r = req(vec![("user", "hi")], false);
        assert!(m.matches(&ctx(&r, &[])));
    }

    #[test]
    fn stream_matcher_true_matches_stream_true() {
        let m = CompiledMatchers::compile(&Matchers {
            stream: Some(true),
            ..Matchers::default()
        })
        .unwrap();
        let r_stream = req(vec![("user", "x")], true);
        let r_nostream = req(vec![("user", "x")], false);
        assert!(m.matches(&ctx(&r_stream, &[])));
        assert!(!m.matches(&ctx(&r_nostream, &[])));
    }

    #[test]
    fn prompt_tokens_gte_threshold() {
        let m = CompiledMatchers::compile(&Matchers {
            prompt_tokens_gte: Some(10),
            ..Matchers::default()
        })
        .unwrap();
        let short = req(vec![("user", "hi")], false);
        let long = req(vec![("user", &"x".repeat(100))], false);
        assert!(!m.matches(&ctx(&short, &[])));
        assert!(m.matches(&ctx(&long, &[])));
    }

    #[test]
    fn prompt_tokens_lte_threshold() {
        let m = CompiledMatchers::compile(&Matchers {
            prompt_tokens_lte: Some(5),
            ..Matchers::default()
        })
        .unwrap();
        let short = req(vec![("user", "hi")], false);
        let long = req(vec![("user", &"x".repeat(100))], false);
        assert!(m.matches(&ctx(&short, &[])));
        assert!(!m.matches(&ctx(&long, &[])));
    }

    #[test]
    fn has_system_prompt_matcher() {
        let m = CompiledMatchers::compile(&Matchers {
            has_system_prompt: Some(true),
            ..Matchers::default()
        })
        .unwrap();
        let with_sys = req(vec![("system", "you are helpful"), ("user", "hi")], false);
        let without = req(vec![("user", "hi")], false);
        assert!(m.matches(&ctx(&with_sys, &[])));
        assert!(!m.matches(&ctx(&without, &[])));
    }

    #[test]
    fn content_regex_matches_last_user_message() {
        let m = CompiledMatchers::compile(&Matchers {
            content_regex: Some(r"(?i)code".into()),
            ..Matchers::default()
        })
        .unwrap();
        let code_req = req(vec![("user", "write some CODE for me")], false);
        let normal = req(vec![("user", "hello there")], false);
        assert!(m.matches(&ctx(&code_req, &[])));
        assert!(!m.matches(&ctx(&normal, &[])));
    }

    #[test]
    fn header_equals_matcher_case_insensitive_name() {
        let m = CompiledMatchers::compile(&Matchers {
            header_equals: [("X-Flarion-Route".to_string(), "fast".to_string())]
                .into_iter()
                .collect(),
            ..Matchers::default()
        })
        .unwrap();
        let r = req(vec![("user", "x")], false);
        assert!(m.matches(&ctx(&r, &[("x-flarion-route", "fast")])));
        assert!(!m.matches(&ctx(&r, &[("x-flarion-route", "slow")])));
        assert!(!m.matches(&ctx(&r, &[])));
    }

    #[test]
    fn message_count_thresholds() {
        let m = CompiledMatchers::compile(&Matchers {
            message_count_gte: Some(3),
            ..Matchers::default()
        })
        .unwrap();
        let single = req(vec![("user", "x")], false);
        let many = req(
            vec![("user", "a"), ("assistant", "b"), ("user", "c")],
            false,
        );
        assert!(!m.matches(&ctx(&single, &[])));
        assert!(m.matches(&ctx(&many, &[])));
    }

    #[test]
    fn multiple_matchers_and_combined() {
        let m = CompiledMatchers::compile(&Matchers {
            stream: Some(true),
            prompt_tokens_gte: Some(5),
            ..Matchers::default()
        })
        .unwrap();
        let r_pass = req(vec![("user", &"x".repeat(100))], true);
        let r_fail_stream = req(vec![("user", &"x".repeat(100))], false);
        let r_fail_tokens = req(vec![("user", "hi")], true);
        assert!(m.matches(&ctx(&r_pass, &[])));
        assert!(!m.matches(&ctx(&r_fail_stream, &[])));
        assert!(!m.matches(&ctx(&r_fail_tokens, &[])));
    }

    #[test]
    fn invalid_regex_rejected_at_compile() {
        let m = Matchers {
            content_regex: Some("(unclosed".into()),
            ..Matchers::default()
        };
        assert!(CompiledMatchers::compile(&m).is_err());
    }
}
