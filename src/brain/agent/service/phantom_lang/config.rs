//! [`LangConfig`] and the six embedded language tables, parsed once on
//! first use. A TOML syntax error fails the build via `include_str!`.

use serde::Deserialize;
use std::sync::LazyLock;

/// Language-specific phantom detection configuration.
#[derive(Debug, Clone, Deserialize)]
pub struct LangConfig {
    #[serde(default)]
    pub intent_phrases: Vec<String>,
    #[serde(default)]
    pub action_verbs: Vec<String>,
    #[serde(default)]
    pub line_start_re: String,
    /// Matches a brief present-continuous work announcement that ends the
    /// turn with no tool call — e.g. "Running checks now.", "Checking the
    /// logs…". These fall under the no-tools detector's length floor and
    /// aren't "I'll / let me / I'm going to" phrases, so they need their own
    /// pattern. Anchored and ending in an imminence marker (now / … / :) to
    /// avoid matching ordinary sentences that merely open with a gerund.
    #[serde(default)]
    pub work_announcement_re: String,
    #[serde(default)]
    pub completion_claims: Vec<String>,
    /// Phrases presenting a named command as ALREADY RUN, e.g. "ran",
    /// "checked with", "output above". Required by the uncalled-command check
    /// (#789): the command itself is language-neutral, but the framing that
    /// turns a proposal into a claim is not, and an English-only list let a
    /// fabrication in any other language through untouched.
    #[serde(default)]
    pub executed_framings: Vec<String>,
    /// Generic first-person intent, e.g. `let me <verb>` / `I'll <verb>`,
    /// with the verb captured in group 1 rather than enumerated. The phrase
    /// list is an allowlist of specific verb pairings, so every verb nobody
    /// thought to add slips through: `let me execute the full flow now` ended
    /// a turn with no tool call because `let me execute` was not among the
    /// 701 entries. Verbs that read as speech rather than action
    /// (`let me know`) are filtered by `intent_verb_exclusions`.
    #[serde(default)]
    pub generic_intent_re: String,
    /// Verbs that follow the construction above without promising work:
    /// `let me know` addresses the user, `I'll be happy to` is not an action.
    #[serde(default)]
    pub intent_verb_exclusions: Vec<String>,
    /// Gerund-led plan announcement with NO imminence marker, sequenced
    /// with then / before — "Setting up the plan, then mapping every call
    /// site before touching anything." `work_announcement_re` enumerates
    /// execution verbs and demands a trailing now / … / :, `gerund_re`
    /// demands a leading "Now"; a turn that announces the PREPARATION for
    /// work satisfied neither and was delivered as if it were an answer
    /// (#1261). Consumed by the zero-tool gate only.
    #[serde(default)]
    pub plan_announcement_re: String,
    /// Bare-participle announcement carrying its OWN object pronoun, with no
    /// imminence marker and no sequencing: "Reading them, with mtimes so I
    /// know each postdates the tree it claims to cover." Every existing arm
    /// needs something this clause does not have — `work_announcement_re` and
    /// `gerund_re` a now / … / :, `plan_announcement_re` a then / before /
    /// after — so the marker-less form shipped as a finished answer while
    /// announcing unfinished work (#1694). Group 1 MUST capture the rest of
    /// the clause; the announcement/statement split reads it.
    ///
    /// The construction is language-specific in more than vocabulary: the
    /// clitic sits after and separate (en, ru), enclitic behind a hyphen (pt),
    /// fused with an accent shift (es), PROCLITIC before the verb (fr), or as
    /// a fused suffix (id). Empty for a language whose split cannot be
    /// discriminated safely — every consumer guards `is_empty()` first, so an
    /// absent arm is inert, not broken.
    #[serde(default)]
    pub participle_object_re: String,
    /// Closed-class words that may follow the pronoun in an ANNOUNCEMENT
    /// (prepositions, conjunctions, numerals, ordinals, modals, fixed
    /// adverbials): "Reading them **against** the log", "Siguiéndolos **paso**
    /// a paso". A tail beginning with anything else is a STATEMENT whose
    /// participle is the subject — "Getting them **took** a while",
    /// "Leyéndolos **reveló** el fallo" — and must not fire.
    ///
    /// This is deliberately a closed class. The split used to be specified as
    /// a copula list, which is both open in the general case and insufficient:
    /// the statements above carry no copula at all, and French "En les
    /// lisant, on comprend vite" likewise has none, so no copula list can
    /// separate them. Verb predicates are an open class and enumerating them
    /// never converges (#1122); the word class AFTER the pronoun does.
    #[serde(default)]
    pub announcement_tail_words: Vec<String>,
    /// Verbs and prepositions that make a following BARE tool name the
    /// object of an action the model attributes to itself: "setting up the
    /// plan", "running bash". A multi-word name proves tool usage on its
    /// own; `plan` / `bash` / `grep` / `ls` are ordinary words and prove it
    /// only in this frame (#1262).
    #[serde(default)]
    pub tool_use_markers: Vec<String>,
    /// Nouns that name the preceding word as a tool ("the plan tool").
    #[serde(default)]
    pub tool_nouns: Vec<String>,
    /// Determiners allowed between a marker and the name. At most one:
    /// anything else in between means the name is not the marker's object,
    /// which is what keeps "run it in bash" (an instruction to the user)
    /// out. Empty for languages without articles.
    #[serde(default)]
    pub tool_name_determiners: Vec<String>,
    #[serde(default)]
    pub gerund_re: String,
    #[serde(default)]
    pub trailing_colon_re: String,
    #[serde(default)]
    pub now_imperative_re: String,
    #[serde(default)]
    pub numbered_steps_re: String,
    #[serde(default)]
    pub past_tense_standalone_re: String,
    #[serde(default)]
    pub path_re: String,
    #[serde(default)]
    pub ext_re: String,
    #[serde(default)]
    pub backtick_code_re: String,
    /// Assertions that a visual/media result was produced or delivered
    /// ("there it is", "generated it", "the edited image", ...). Paired with
    /// `media_context_words` to catch image-generation hallucination in any
    /// language (#747) — a claim of a delivered image with no `<<IMG:>>` marker.
    #[serde(default)]
    pub media_delivery_phrases: Vec<String>,
    /// Words marking a claimed result as visual/image work ("image", "photo",
    /// "background", "brightness", ...). Paired with `media_delivery_phrases`.
    #[serde(default)]
    pub media_context_words: Vec<String>,
    /// Claims that a FILE or DOCUMENT was sent ("file sent", "attached the
    /// file", "sent the report"). Paired with `file_context_words` to catch a
    /// delivery claim in a turn that never invoked a document-sending tool
    /// (#825). Document sending is newer than image generation, so the media
    /// checks (#747) did not cover it.
    #[serde(default)]
    pub file_delivery_phrases: Vec<String>,
    /// Words marking the claimed deliverable as a file rather than a message
    /// ("file", "document", "report", ".md"). Paired with
    /// `file_delivery_phrases`.
    #[serde(default)]
    pub file_context_words: Vec<String>,
}

/// Embedded TOML content (compile-time validated).
const EN_TOML: &str = include_str!("en.toml");
const RU_TOML: &str = include_str!("ru.toml");
const ES_TOML: &str = include_str!("es.toml");
const PT_TOML: &str = include_str!("pt.toml");
const FR_TOML: &str = include_str!("fr.toml");
const ID_TOML: &str = include_str!("id.toml");

pub(crate) static LANG_EN: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(EN_TOML).expect("BUG: en.toml failed to parse at runtime"));
pub(crate) static LANG_RU: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(RU_TOML).expect("BUG: ru.toml failed to parse at runtime"));
pub(crate) static LANG_ES: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(ES_TOML).expect("BUG: es.toml failed to parse at runtime"));
pub(crate) static LANG_PT: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(PT_TOML).expect("BUG: pt.toml failed to parse at runtime"));
pub(crate) static LANG_FR: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(FR_TOML).expect("BUG: fr.toml failed to parse at runtime"));
pub(crate) static LANG_ID: LazyLock<LangConfig> =
    LazyLock::new(|| toml::from_str(ID_TOML).expect("BUG: id.toml failed to parse at runtime"));
