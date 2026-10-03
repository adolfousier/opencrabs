//! Discord Poll Spec (#1848)
//!
//! Validates and clamps a poll request to what the Discord Poll API accepts, so
//! the tool surface can build a native poll on par with Telegram's `send_poll`
//! and WhatsApp's `send_poll`. The builder call itself is typestate-guarded
//! (`CreatePoll` walks `NeedsQuestion` -> `NeedsAnswers` -> `NeedsDuration` ->
//! `Ready`, serenity 0.12.5 `builder/create_poll.rs`), so a malformed poll
//! cannot compile. Everything that can be wrong at runtime lives in the limits
//! below, which is why they are pure functions here and unit-tested.
//!
//! Limits are quoted from the primary source, discord-api-docs
//! `developers/resources/poll.mdx` (Poll Create Request Object + Poll Media
//! Object), verified 2026-10-02:
//!
//! | Limit | Value | Source line |
//! |---|---|---|
//! | Question text | 300 chars max | "The maximum length of `text` is 300 for the question" |
//! | Answer text | 55 chars max | "and 55 for any answer" |
//! | Answer count | up to 10 | "Each of the answers available in the poll, up to 10" |
//! | Duration | hours, up to 32 days, defaults to 24 | "Number of hours the poll should be open for, up to 32 days" |
//! | Multiselect | defaults to false | "Whether a user can select multiple answers. Defaults to false" |
//!
//! Only `text` is supported for the question, and answers carry an optional
//! emoji that this module deliberately does not expose: the Telegram and
//! WhatsApp poll params are plain option strings, and emoji would be a third
//! shape on a surface the agent already knows.

use serenity::builder::CreatePollAnswer;

/// Max characters of Discord poll question text.
pub const QUESTION_MAX: usize = 300;
/// Max characters of one Discord poll answer label.
pub const ANSWER_MAX: usize = 55;
/// Max answers per Discord poll.
pub const MAX_ANSWERS: usize = 10;
/// Minimum answers we accept. Discord permits one, but a one-option poll is not
/// a question, and the sibling channel we mirror requires two (Telegram
/// `telegram_send.rs:1233`, "must have at least 2 options"), so parity wins.
pub const MIN_ANSWERS: usize = 2;
/// Discord's default poll duration in hours when the caller passes none.
pub const DEFAULT_DURATION_HOURS: u16 = 24;
/// 32 days expressed in hours, the platform ceiling.
pub const MAX_DURATION_HOURS: u16 = 768;

/// Why a poll request was refused outright, as opposed to clamped.
#[derive(Debug, PartialEq, Eq)]
pub enum PollError {
    /// Question was empty, whitespace, or became empty after trimming.
    EmptyQuestion,
    /// Fewer than [`MIN_ANSWERS`] usable answers were supplied.
    TooFewAnswers(usize),
}

impl PollError {
    /// Agent-facing message. Kept separate from the display impl so the tool
    /// arm can wrap it in `ToolResult::error` without a `String` round trip.
    pub fn message(&self) -> String {
        match self {
            Self::EmptyQuestion => "send_poll requires a non-empty 'poll_question'.".to_string(),
            Self::TooFewAnswers(n) => format!(
                "send_poll needs at least {MIN_ANSWERS} non-empty options in 'poll_options', got {n}."
            ),
        }
    }
}

/// A poll request that is safe to hand to the Discord API.
#[derive(Debug, PartialEq, Eq)]
pub struct PollSpec {
    /// Trimmed and truncated to [`QUESTION_MAX`] chars.
    pub question: String,
    /// Blank entries removed, each truncated to [`ANSWER_MAX`] chars, capped at
    /// [`MAX_ANSWERS`].
    pub answers: Vec<String>,
    /// Clamped into `1..=MAX_DURATION_HOURS`.
    pub duration_hours: u16,
    /// Passed straight through.
    pub allow_multiselect: bool,
    /// Answers dropped by the [`MAX_ANSWERS`] cap. Reported back to the caller
    /// so a dropped option is never silent.
    pub dropped_answers: usize,
    /// True when the requested duration fell outside `1..=768` hours.
    pub duration_clamped: bool,
}

/// Truncate to `max` characters on a `char` boundary.
///
/// Byte slicing would panic on the multibyte input agents actually send
/// (emoji, accented text), so this counts `chars`, not bytes.
pub fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    s.chars().take(max).collect()
}

/// Validate and clamp a raw poll request.
///
/// `duration_hours` is `None` when the caller omitted it (Discord's own
/// default of 24 applies) and `Some(0)` or `Some(>768)` when the caller asked
/// for something the platform will not carry, in which case it is clamped and
/// [`PollSpec::duration_clamped`] is set so the tool result can say so.
pub fn build_spec(
    question: &str,
    options: &[String],
    duration_hours: Option<i64>,
    allow_multiselect: bool,
) -> Result<PollSpec, PollError> {
    let question = truncate_chars(question.trim(), QUESTION_MAX);
    if question.is_empty() {
        return Err(PollError::EmptyQuestion);
    }

    let usable: Vec<String> = options
        .iter()
        .map(|o| o.trim())
        .filter(|o| !o.is_empty())
        .map(|o| truncate_chars(o, ANSWER_MAX))
        .collect();
    if usable.len() < MIN_ANSWERS {
        return Err(PollError::TooFewAnswers(usable.len()));
    }

    let kept = usable.len().min(MAX_ANSWERS);
    let dropped_answers = usable.len() - kept;
    let answers = usable.into_iter().take(kept).collect();

    let (duration_hours, duration_clamped) = match duration_hours {
        None => (DEFAULT_DURATION_HOURS, false),
        Some(h) if (1..=MAX_DURATION_HOURS as i64).contains(&h) => (h as u16, false),
        Some(h) => (if h < 1 { 1 } else { MAX_DURATION_HOURS }, true),
    };

    Ok(PollSpec {
        question,
        answers,
        duration_hours,
        allow_multiselect,
        dropped_answers,
        duration_clamped,
    })
}

/// Turn the validated answers into serenity answer builders, text-only.
pub fn answer_builders(spec: &PollSpec) -> Vec<CreatePollAnswer> {
    spec.answers
        .iter()
        .map(|a| CreatePollAnswer::new().text(a.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serenity::builder::CreatePoll;

    fn opts(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn empty_question_is_refused() {
        assert_eq!(
            build_spec("   ", &opts(&["a", "b"]), None, false),
            Err(PollError::EmptyQuestion)
        );
    }

    #[test]
    fn fewer_than_two_usable_answers_is_refused() {
        assert_eq!(
            build_spec("Q", &opts(&["only one"]), None, false),
            Err(PollError::TooFewAnswers(1))
        );
        // Blanks do not count toward the minimum, so this is still one.
        assert_eq!(
            build_spec("Q", &opts(&["real", "", "   "]), None, false),
            Err(PollError::TooFewAnswers(1))
        );
    }

    #[test]
    fn blank_options_are_dropped_not_sent() {
        let spec = build_spec("Q", &opts(&["a", "", "  ", "b"]), None, false).unwrap();
        assert_eq!(spec.answers, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(spec.dropped_answers, 0);
    }

    #[test]
    fn more_than_ten_answers_is_capped_and_reported() {
        let twelve: Vec<String> = (1..=12).map(|i| format!("opt{i}")).collect();
        let spec = build_spec("Q", &twelve, None, false).unwrap();
        assert_eq!(spec.answers.len(), MAX_ANSWERS);
        assert_eq!(spec.dropped_answers, 2);
        assert_eq!(spec.answers.first().unwrap(), "opt1");
        assert_eq!(spec.answers.last().unwrap(), "opt10");
    }

    #[test]
    fn exactly_ten_answers_is_not_capped() {
        let ten: Vec<String> = (1..=10).map(|i| format!("o{i}")).collect();
        let spec = build_spec("Q", &ten, None, false).unwrap();
        assert_eq!(spec.answers.len(), 10);
        assert_eq!(spec.dropped_answers, 0);
    }

    #[test]
    fn long_answer_truncates_to_55_chars() {
        let long = "x".repeat(90);
        let spec = build_spec("Q", &opts(&["short", &long]), None, false).unwrap();
        assert_eq!(spec.answers[1].chars().count(), ANSWER_MAX);
    }

    #[test]
    fn long_question_truncates_to_300_chars() {
        let long = "y".repeat(400);
        let spec = build_spec(&long, &opts(&["a", "b"]), None, false).unwrap();
        assert_eq!(spec.question.chars().count(), QUESTION_MAX);
    }

    #[test]
    fn truncation_is_char_safe_on_multibyte_input() {
        // 200 emoji is 800 bytes; a byte slice here would panic or split a
        // code point. 100 of them crosses the 300-char boundary.
        let q = "🦀".repeat(350);
        let spec = build_spec(&q, &opts(&["a", "b"]), None, false).unwrap();
        assert_eq!(spec.question.chars().count(), QUESTION_MAX);
        assert!(spec.question.chars().all(|c| c == '🦀'));
        // Round trips as valid UTF-8, which is the actual hazard.
        assert!(std::str::from_utf8(spec.question.as_bytes()).is_ok());
    }

    #[test]
    fn question_is_trimmed() {
        let spec = build_spec("  real question  ", &opts(&["a", "b"]), None, false).unwrap();
        assert_eq!(spec.question, "real question");
    }

    #[test]
    fn absent_duration_takes_discords_default() {
        let spec = build_spec("Q", &opts(&["a", "b"]), None, false).unwrap();
        assert_eq!(spec.duration_hours, DEFAULT_DURATION_HOURS);
        assert!(!spec.duration_clamped);
    }

    #[test]
    fn in_range_duration_passes_through() {
        let spec = build_spec("Q", &opts(&["a", "b"]), Some(48), false).unwrap();
        assert_eq!(spec.duration_hours, 48);
        assert!(!spec.duration_clamped);
        let spec = build_spec("Q", &opts(&["a", "b"]), Some(768), false).unwrap();
        assert_eq!(spec.duration_hours, 768);
        assert!(!spec.duration_clamped);
    }

    #[test]
    fn zero_or_negative_duration_clamps_up_to_one_hour() {
        for h in [0, -1, -1000] {
            let spec = build_spec("Q", &opts(&["a", "b"]), Some(h), false).unwrap();
            assert_eq!(spec.duration_hours, 1, "duration {h}");
            assert!(spec.duration_clamped);
        }
    }

    #[test]
    fn oversized_duration_clamps_to_32_days() {
        for h in [769, 1000, i64::MAX] {
            let spec = build_spec("Q", &opts(&["a", "b"]), Some(h), false).unwrap();
            assert_eq!(spec.duration_hours, MAX_DURATION_HOURS, "duration {h}");
            assert!(spec.duration_clamped);
        }
    }

    #[test]
    fn multiselect_passes_through() {
        let spec = build_spec("Q", &opts(&["a", "b"]), None, true).unwrap();
        assert!(spec.allow_multiselect);
        let spec = build_spec("Q", &opts(&["a", "b"]), None, false).unwrap();
        assert!(!spec.allow_multiselect);
    }

    #[test]
    fn answer_builders_mirror_the_spec() {
        let spec = build_spec("Q", &opts(&["a", "b", "c"]), None, false).unwrap();
        let builders = answer_builders(&spec);
        assert_eq!(builders.len(), 3);
        // The builder serialises to {"poll_media":{"text":...}}, so the labels
        // must survive the round trip or the poll ships blank answers.
        let json = serde_json::to_value(&builders).unwrap();
        assert_eq!(json[0]["poll_media"]["text"], serde_json::json!("a"));
        assert_eq!(json[2]["poll_media"]["text"], serde_json::json!("c"));
    }

    #[test]
    fn spec_serialises_to_the_poll_create_request_shape() {
        let spec = build_spec("Cats or dogs?", &opts(&["Cats", "Dogs"]), Some(12), true).unwrap();
        let builders = answer_builders(&spec);
        let mut poll = CreatePoll::new()
            .question(spec.question.clone())
            .answers(builders)
            .duration(std::time::Duration::from_secs(
                u64::from(spec.duration_hours) * 3600,
            ));
        if spec.allow_multiselect {
            poll = poll.allow_multiselect();
        }
        let json = serde_json::to_value(&poll).unwrap();
        assert_eq!(json["question"]["text"], serde_json::json!("Cats or dogs?"));
        assert_eq!(json["answers"].as_array().unwrap().len(), 2);
        assert_eq!(json["duration"], serde_json::json!(12));
        assert_eq!(json["allow_multiselect"], serde_json::json!(true));
    }

    #[test]
    fn error_messages_name_the_param_and_the_count() {
        assert_eq!(
            PollError::EmptyQuestion.message(),
            "send_poll requires a non-empty 'poll_question'."
        );
        assert_eq!(
            PollError::TooFewAnswers(1).message(),
            "send_poll needs at least 2 non-empty options in 'poll_options', got 1."
        );
    }
}
