//! `kluris search`: BM25F over heading chunks (research-05 §3, §6 rung 1),
//! with v2's substring hits always kept (research-03 §2.6).
//! `parity/test_search.py` checks the v2 side against the v2.28.11 oracle.

use std::fs;
use std::path::{Path, PathBuf};

use kluris_core::brain::store::LocalGitStore;
use kluris_core::search::bm25::{B, FIELD_WEIGHTS, Field, Index, K1, QueryTerm, Unit};
use kluris_core::search::chunk::{WINDOW_WORDS, chunks};
use kluris_core::search::items::{ItemKind, collect_items, py_title, v2_match};
use kluris_core::search::snippet::snippet;
use kluris_core::search::tokenize::{fold, query_terms, tokens};
use kluris_core::search::{Hit, Language, SearchRequest, detect_language, parse_limit, search};
use tempfile::TempDir;

struct Brain {
    _tmp: TempDir,
    root: PathBuf,
}

impl Brain {
    fn new() -> Brain {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().join("brain");
        fs::create_dir_all(&root).unwrap();
        Brain { _tmp: tmp, root }
    }

    fn file(&self, rel: &str, text: &str) -> &Brain {
        let path = self.root.join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        self
    }

    fn store(&self) -> LocalGitStore {
        LocalGitStore::open(&self.root)
    }

    fn search(&self, query: &str) -> Vec<Hit> {
        self.search_with(SearchRequest::new(query))
    }

    fn search_with(&self, request: SearchRequest) -> Vec<Hit> {
        search(&self.store(), &request).hits
    }
}

fn neuron(title: &str, body: &str) -> String {
    format!(
        "---\nparent: map.md\ncreated: 2026-01-01\nupdated: 2026-01-02\n---\n# {title}\n\n{body}\n"
    )
}

fn files(hits: &[Hit]) -> Vec<&str> {
    hits.iter().map(|h| h.file.as_str()).collect()
}

fn words(tokens: &[kluris_core::search::tokenize::Token]) -> Vec<&str> {
    tokens.iter().map(|t| t.word.as_str()).collect()
}

fn terms(tokens: &[kluris_core::search::tokenize::Token]) -> Vec<&str> {
    tokens.iter().map(|t| t.term.as_str()).collect()
}

// --- tokenizer ---------------------------------------------------------------

#[test]
fn fold_removes_diacritics_and_compatibility_forms() {
    assert_eq!(fold("prețul ședinței"), "pretul sedintei");
    assert_eq!(fold("Ångström café"), "angstrom cafe");
    // NFD input (macOS file names) folds the same way.
    assert_eq!(fold("s\u{0326}edint\u{0326}a"), "sedinta");
    assert_eq!(fold("ﬁle ＡＢＣ"), "file abc");
}

#[test]
fn tokens_split_on_unicode_word_boundaries_and_punctuation() {
    let en = Language::English;
    assert_eq!(
        words(&tokens("KLURIS_NO_PROMPT=1", en)),
        ["kluris", "no", "prompt", "1"]
    );
    assert_eq!(
        words(&tokens("POST /api/v1/imports/users", en)),
        ["post", "api", "v1", "imports", "users"]
    );
    assert_eq!(
        words(&tokens("kluris.yml darwin-aarch64", en)),
        ["kluris", "yml", "darwin", "aarch64"]
    );
    // In-word apostrophes join; NFD marks stay inside their word.
    assert_eq!(words(&tokens("don't it’s", en)), ["dont", "its"]);
    assert_eq!(words(&tokens("s\u{0326}edint\u{0326}a", en)), ["sedinta"]);
    assert!(tokens("!!!! — ... ``", en).is_empty());
}

#[test]
fn tokens_carry_byte_offsets_into_the_original_text() {
    let text = "Prețul — ședinței";
    let toks = tokens(text, Language::English);
    assert_eq!(&text[toks[0].start..toks[0].end], "Prețul");
    assert_eq!(&text[toks[1].start..toks[1].end], "ședinței");
}

#[test]
fn stop_words_are_flagged_and_dropped_from_queries() {
    let toks = tokens("how do I rotate the keys", Language::English);
    assert_eq!(
        toks.iter().map(|t| t.stop).collect::<Vec<_>>(),
        [true, true, true, false, true, false]
    );
    assert_eq!(
        words(&query_terms("how do I rotate the keys", Language::English)),
        ["rotate", "keys"]
    );
}

#[test]
fn a_query_of_only_stop_words_keeps_them() {
    assert_eq!(
        words(&query_terms("how do I", Language::English)),
        ["how", "do", "i"]
    );
    assert!(query_terms("!!!", Language::English).is_empty());
}

#[test]
fn romanian_stop_words_apply_to_romanian_brains_only() {
    // `ca` is Romanian for "as"; in an English brain it is the CA in "CA bundle".
    assert_eq!(
        words(&query_terms("private CA bundle", Language::English)),
        ["private", "ca", "bundle"]
    );
    assert_eq!(
        words(&query_terms(
            "ce mail primește consilierul",
            Language::Romanian
        )),
        ["mail", "primeste", "consilierul"]
    );
    // English stop words still drop in a Romanian brain (English questions).
    assert_eq!(
        words(&query_terms("how long do we keep data", Language::Romanian)),
        ["long", "keep", "data"]
    );
}

// --- stemming --------------------------------------------------------------------

#[test]
fn english_stemming_joins_word_forms() {
    let en = Language::English;
    assert_eq!(
        terms(&tokens("connecting connection connected", en)),
        ["connect", "connect", "connect"]
    );
    assert_eq!(terms(&tokens("checkpoints", en)), ["checkpoint"]);
}

#[test]
fn romanian_stemming_sees_folded_words_so_missing_diacritics_still_match() {
    // Snowball's Romanian rules key on ă, â, î, ş, ţ, so a word typed without
    // them would stem differently (`consimtamant` vs `consimţământ`); the
    // stemmer therefore always gets the folded word.
    let ro = Language::Romanian;
    for (with, without) in [
        ("prețul ședinței", "pretul sedintei"),
        ("consimțământul", "consimtamantul"),
        ("ședințe", "şedinţe"),
    ] {
        assert_eq!(
            terms(&tokens(with, ro)),
            terms(&tokens(without, ro)),
            "{with}"
        );
    }
    assert_eq!(
        terms(&tokens("consimțământ consimțământul", ro)),
        ["consimtam", "consimtam"]
    );
    // The Romanian stemmer joins forms the English one leaves apart.
    assert_eq!(
        terms(&tokens("programarea programare", ro)),
        ["program", "program"]
    );
    let en = tokens("programarea programare", Language::English);
    assert_ne!(en[0].term, en[1].term);
}

// --- language detection ----------------------------------------------------------

#[test]
fn language_is_detected_from_stop_word_counts() {
    assert_eq!(
        detect_language([
            "Consimțământul este dat de utilizator și este păstrat în baza de date, pentru că legea cere asta."
        ]),
        Language::Romanian
    );
    assert_eq!(
        detect_language([
            "The consent is given by the user and is kept in the database, as the law requires it."
        ]),
        Language::English
    );
    assert_eq!(detect_language(Vec::<&str>::new()), Language::English);
    assert_eq!(detect_language(["kluris pack docker"]), Language::English);
}

#[test]
fn language_codes() {
    assert_eq!(Language::from_code("en"), Some(Language::English));
    assert_eq!(Language::from_code("ro"), Some(Language::Romanian));
    assert_eq!(Language::from_code("RO"), Some(Language::Romanian));
    assert_eq!(Language::from_code("fr"), None);
    assert_eq!(Language::Romanian.code(), "ro");
}

#[test]
fn search_detects_the_brain_language_and_honours_an_override() {
    let b = Brain::new();
    b.file(
        "operare/preturi.md",
        &neuron("Prețuri", "Prețul unei ședințe se schimbă în fișierul de configurare și este publicat după verificare."),
    );
    let store = b.store();
    assert_eq!(
        search(&store, &SearchRequest::new("pret")).language,
        Language::Romanian
    );
    let mut req = SearchRequest::new("pret");
    req.language = Some(Language::English);
    assert_eq!(search(&store, &req).language, Language::English);
}

// --- chunking --------------------------------------------------------------------

#[test]
fn chunks_split_at_level_two_and_three_headings() {
    let body = "# Title\n\nintro words\n\n## Setup\n\nsetup words\n\n### Details\n\ndetail words\n\n#### Deep\n\ndeep words\n";
    let c = chunks(body);
    let heads: Vec<Option<&str>> = c.iter().map(|c| c.heading.as_deref()).collect();
    assert_eq!(heads, [None, Some("Setup"), Some("Details")]);
    assert_eq!(c[0].line, 0);
    assert_eq!(c[1].line, 4);
    assert_eq!(c[2].line, 8);
    assert!(
        body[c[2].start..c[2].end].contains("deep words"),
        "#### stays inside its section"
    );
    assert!(!body[c[0].start..c[0].end].contains("setup"));
}

#[test]
fn headings_inside_code_fences_do_not_split() {
    let body = "intro\n\n```bash\n## not a heading\n```\n\n## Real\n\ntext\n";
    let c = chunks(body);
    assert_eq!(
        c.iter().map(|c| c.heading.as_deref()).collect::<Vec<_>>(),
        [None, Some("Real")]
    );
}

#[test]
fn long_sections_are_windowed_on_line_boundaries() {
    let line = "alpha beta gamma delta epsilon zeta eta theta iota kappa\n"; // 10 words
    let body = format!("## Long\n\n{}", line.repeat(50)); // 500 words
    let c = chunks(&body);
    assert_eq!(c.len(), 3, "500 words in windows of {WINDOW_WORDS}");
    assert!(c.iter().all(|c| c.heading.as_deref() == Some("Long")));
    assert_eq!(c[0].line, 0);
    assert_eq!(c[1].line, 2 + WINDOW_WORDS / 10);
    for chunk in &c {
        let text = &body[chunk.start..chunk.end];
        assert!(text.is_empty() || text.ends_with('\n') || chunk.end == body.len());
        assert!(text.split_whitespace().count() <= WINDOW_WORDS + 10);
    }
}

#[test]
fn an_empty_body_is_one_chunk() {
    let c = chunks("");
    assert_eq!(c.len(), 1);
    assert_eq!(
        (c[0].heading.clone(), c[0].line, c[0].start, c[0].end),
        (None, 0, 0, 0)
    );
}

#[test]
fn title_tags_and_path_are_repeated_in_every_chunk() {
    let b = Brain::new();
    b.file(
        "ops/backups.md",
        "---\ntags: [postgres]\n---\n# Backups\n\n## Roles\n\nroles text\n\n## Schedule\n\nnightly text\n",
    );
    // A query on the title and a body word of the second section finds that
    // section, so the title is in that chunk too.
    let hits = b.search("backups nightly");
    assert_eq!(files(&hits), ["ops/backups.md"]);
    assert_eq!(hits[0].heading.as_deref(), Some("Schedule"));
    let hits = b.search("postgres roles");
    assert_eq!(hits[0].heading.as_deref(), Some("Roles"));
}

#[test]
fn line_is_the_one_based_line_of_the_best_chunk_in_the_file() {
    let b = Brain::new();
    // Line 1 `---` ... line 5 `---`, line 6 `# T`, line 8 `## A`, line 12 `## B`.
    b.file(
        "l/n.md",
        "---\nparent: map.md\ncreated: 2026-01-01\nupdated: 2026-01-02\n---\n# T\n\n## A\n\nfirst\n\n## B\n\nsecond zebra\n",
    );
    let hits = b.search("zebra");
    assert_eq!(hits[0].line, Some(12));
    assert_eq!(hits[0].heading.as_deref(), Some("B"));
    // CRLF files count lines the same way.
    let b = Brain::new();
    b.file(
        "l/n.md",
        "---\r\nupdated: 2026-01-02\r\n---\r\n\r\n# T\r\n\r\n## B\r\n\r\nzebra\r\n",
    );
    assert_eq!(b.search("zebra")[0].line, Some(7));
}

// --- BM25F ------------------------------------------------------------------------

fn unit(title: &[&str], body: &[&str]) -> Unit {
    let mut u = Unit::default();
    u.fields[Field::Title as usize] = title.iter().map(|s| s.to_string()).collect();
    u.fields[Field::Body as usize] = body.iter().map(|s| s.to_string()).collect();
    u
}

#[test]
fn field_weights_are_the_measured_ones() {
    assert_eq!(FIELD_WEIGHTS, [3.0, 2.0, 1.5, 1.5, 1.0]);
    assert_eq!((K1, B), (1.2, 0.75));
}

#[test]
fn bm25f_matches_the_formula() {
    // Two units: "zebra" once in the title of the first, once in the body of
    // the second; both fields of both units are 1 word long except the second
    // unit's title (empty) and the first unit's body (empty).
    let units = [unit(&["zebra"], &[]), unit(&[], &["zebra"])];
    let index = Index::new(&units);
    let scores = index.score(&[QueryTerm::exact("zebra")]);
    let n = 2.0_f64;
    let df = 2.0_f64;
    let idf = (1.0 + (n - df + 0.5) / (df + 0.5)).ln();
    // avg title length 0.5, avg body length 0.5; each field is 1 word.
    let norm = 1.0 - B + B * (1.0 / 0.5);
    let tf_title = FIELD_WEIGHTS[Field::Title as usize] / norm;
    let tf_body = FIELD_WEIGHTS[Field::Body as usize] / norm;
    let expect = |tf: f64| idf * tf / (K1 + tf);
    assert!((scores[0] - expect(tf_title)).abs() < 1e-12, "{scores:?}");
    assert!((scores[1] - expect(tf_body)).abs() < 1e-12, "{scores:?}");
    assert!(scores[0] > scores[1]);
}

#[test]
fn a_title_hit_outranks_a_body_hit() {
    let b = Brain::new();
    b.file(
        "a/body.md",
        &neuron("Notes", "The zebra crossing is painted every spring."),
    );
    b.file(
        "a/title.md",
        &neuron("Zebra crossing", "Painted every spring by the city."),
    );
    assert_eq!(files(&b.search("zebra"))[..2], ["a/title.md", "a/body.md"]);
}

#[test]
fn prefix_terms_score_as_the_best_of_their_expansions() {
    let units = [unit(&[], &["authent", "authent"]), unit(&[], &["author"])];
    let index = Index::new(&units);
    assert!(index.has_term("authent"));
    assert!(!index.has_term("auth"));
    let mut expansions = index.prefix_terms("auth");
    expansions.sort();
    assert_eq!(expansions, ["authent", "author"]);
    let any = index.score(&[QueryTerm::any(vec!["authent".into(), "author".into()])]);
    let one = index.score(&[QueryTerm::exact("authent")]);
    assert!((any[0] - one[0]).abs() < 1e-12, "max, not sum");
    assert!(any[1] > 0.0);
}

#[test]
fn prefix_fallback_finds_longer_words() {
    let b = Brain::new();
    b.file(
        "k/login.md",
        &neuron("Login", "Authentication uses short-lived tokens."),
    );
    b.file("k/other.md", &neuron("Other", "Nothing related here."));
    assert_eq!(files(&b.search("auth")), ["k/login.md"]);
    // Two characters are too short for the fallback (and match no substring
    // in the other neuron).
    assert!(b.search("zq").is_empty());
}

#[test]
fn every_v2_substring_hit_is_kept() {
    let b = Brain::new();
    // `auth` inside `oauth` is no word and no prefix: only v2's substring
    // match finds it.
    b.file(
        "k/oauth.md",
        &neuron("Providers", "We sign in with oauth2 providers."),
    );
    b.file(
        "k/punct.md",
        &neuron("Odd", "Say `!!!! take care` before leaving."),
    );
    let hits = b.search("auth");
    assert_eq!(files(&hits), ["k/oauth.md"]);
    assert!(hits[0].score >= 1, "a returned hit never scores 0");
    assert_eq!(files(&b.search("`!!!! take care`")), ["k/punct.md"]);
    assert_eq!(files(&b.search("!!!!")), ["k/punct.md"]);
}

#[test]
fn bm25_hits_rank_above_substring_only_hits() {
    let b = Brain::new();
    b.file(
        "k/infix.md",
        &neuron("Providers", "oauth oauth oauth oauth"),
    );
    b.file("k/word.md", &neuron("Tokens", "auth tokens"));
    assert_eq!(files(&b.search("auth")), ["k/word.md", "k/infix.md"]);
}

#[test]
fn an_exact_phrase_gets_a_bonus() {
    let b = Brain::new();
    b.file(
        "k/scattered.md",
        &neuron(
            "Release",
            "release release release and notes notes notes for the release train with notes",
        ),
    );
    b.file(
        "k/phrase.md",
        &neuron("Changelog", "Write the release notes before tagging."),
    );
    assert_eq!(files(&b.search("release notes"))[0], "k/phrase.md");
}

#[test]
fn deprecated_neurons_score_half_and_are_never_hidden() {
    let b = Brain::new();
    let body = "The ledger keeps every consent event.";
    b.file("k/new.md", &neuron("Consent ledger", body));
    b.file(
        "k/old.md",
        &format!(
            "---\nstatus: deprecated\nreplaced_by: new.md\nupdated: 2026-01-02\n---\n# Consent ledger\n\n{body}\n"
        ),
    );
    let hits = b.search("consent ledger");
    assert_eq!(files(&hits), ["k/new.md", "k/old.md"]);
    assert!(hits[1].deprecated && !hits[0].deprecated);
    let ratio = hits[1].score as f64 / hits[0].score as f64;
    assert!(
        (ratio - 0.5).abs() < 0.02,
        "{} / {}",
        hits[1].score,
        hits[0].score
    );
    assert_eq!(hits[1].replaced_by.as_deref(), Some("new.md"));
    assert_eq!(hits[0].replaced_by, None);
}

// --- items (v2's searchable items) -----------------------------------------------

#[test]
fn python_title_case() {
    assert_eq!(py_title("my neuron"), "My Neuron");
    assert_eq!(py_title("2nd try"), "2Nd Try");
    assert_eq!(py_title("don't stop"), "Don'T Stop");
    assert_eq!(py_title("ALL CAPS"), "All Caps");
    assert_eq!(py_title("straße"), "Straße");
    assert_eq!(py_title("ßa"), "Ssa");
    assert_eq!(py_title("şedinţa"), "Şedinţa");
}

#[test]
fn items_are_v2s_neurons_glossary_entries_and_brain_md() {
    let b = Brain::new();
    b.file(
        "brain.md",
        "---\nupdated: 2026-01-01\n---\n# demo\n\n## Lobes\n\n- [k](k/map.md)\n",
    );
    b.file(
        "glossary.md",
        "# Glossary\n\n**JWT** -- JSON Web Token\n| CSM | consilier |\n",
    );
    b.file("k/map.md", "# k\n");
    b.file(
        "k/no-title.md",
        "---\nupdated: 2026-01-01\n---\nplain body\n",
    );
    b.file(
        "k/spec.yml",
        "#---\n# title: '  Spec Title  '\n# tags: [api, 7]\n#---\nk: v\n",
    );
    b.file("k/broken.md", "---\na: [\n---\n# Broken\n");
    let items = collect_items(&b.store());
    let rows: Vec<(ItemKind, &str, &str)> = items
        .iter()
        .map(|i| (i.kind, i.file.as_str(), i.title.as_str()))
        .collect();
    assert_eq!(
        rows,
        [
            (ItemKind::Neuron, "k/no-title.md", "No Title"),
            (ItemKind::Neuron, "k/spec.yml", "Spec Title"),
            (ItemKind::Glossary, "glossary.md", "JWT"),
            (ItemKind::Glossary, "glossary.md", "CSM"),
            (ItemKind::BrainMd, "brain.md", "demo"),
        ]
    );
    assert_eq!(items[1].file_type, "yaml");
    assert_eq!(items[2].body, "JSON Web Token");
}

#[test]
fn v2_match_counts_substrings_by_field() {
    let b = Brain::new();
    b.file(
        "auth/oauth.md",
        "---\ntags: [oauth, OAuth2, 5]\n---\n# OAuth flow\n\noauth oauth body\n",
    );
    let items = collect_items(&b.store());
    let m = v2_match(&items[0], "oauth").unwrap();
    // 10 * title + 5 * tags + 3 * path + body.
    // The body is the whole text after the frontmatter, title line included.
    assert_eq!(m.score, 10 + 5 * 2 + 3 + 3);
    assert_eq!(m.fields, ["title", "tag", "path", "body"]);
    assert!(v2_match(&items[0], "zebra").is_none());
}

// --- filters and limit ------------------------------------------------------------

#[test]
fn lobe_and_tag_filters_are_v2s() {
    let b = Brain::new();
    b.file("glossary.md", "**zebra** - an animal\n");
    b.file("projects/foo/a.md", "---\ntags: [api]\n---\n# zebra a\n");
    b.file("projects/foobar/b.md", "---\ntags: [API]\n---\n# zebra b\n");
    b.file("other/c.md", "---\ntags: [api]\n---\n# zebra c\n");
    let mut req = SearchRequest::new("zebra");
    req.lobe = Some("projects/foo/".into());
    assert_eq!(files(&b.search_with(req)), ["projects/foo/a.md"]);
    let mut req = SearchRequest::new("zebra");
    req.tag = Some("api".into());
    let mut got = files(&b.search_with(req))
        .into_iter()
        .map(String::from)
        .collect::<Vec<_>>();
    got.sort();
    assert_eq!(got, ["other/c.md", "projects/foo/a.md"]);
    let mut req = SearchRequest::new("zebra");
    req.lobe = Some("projects".into());
    req.tag = Some("API".into());
    assert_eq!(files(&b.search_with(req)), ["projects/foobar/b.md"]);
}

#[test]
fn limit_caps_results_and_non_positive_limits_return_nothing() {
    let b = Brain::new();
    for i in 0..15 {
        b.file(
            &format!("k/n{i:02}.md"),
            &neuron(&format!("zebra {i}"), "zebra"),
        );
    }
    assert_eq!(b.search("zebra").len(), 10, "default limit 10");
    let mut req = SearchRequest::new("zebra");
    req.limit = 3;
    assert_eq!(b.search_with(req).len(), 3);
    for limit in [0, -1] {
        let mut req = SearchRequest::new("zebra");
        req.limit = limit;
        assert!(b.search_with(req).is_empty());
    }
}

#[test]
fn limits_parse_like_python_int() {
    assert_eq!(parse_limit("10"), Some(10));
    assert_eq!(parse_limit(" +7 "), Some(7));
    assert_eq!(parse_limit("1_000"), Some(1000));
    assert_eq!(parse_limit("-3"), Some(-3));
    assert_eq!(parse_limit("١٢"), Some(12));
    assert_eq!(parse_limit("99999999999999999999999"), Some(i64::MAX));
    assert_eq!(parse_limit("abc"), None);
    assert_eq!(parse_limit("1.5"), None);
    assert_eq!(parse_limit(""), None);
}

// --- results ------------------------------------------------------------------------

#[test]
fn glossary_entries_are_their_own_results() {
    let b = Brain::new();
    b.file(
        "glossary.md",
        "# Glossary\n\n**AIVAT** - a variance reduction technique\n**EV** - expected value\n",
    );
    b.file(
        "k/eval.md",
        &neuron("Evaluation", "We measure with AIVAT every night."),
    );
    let hits = b.search("AIVAT");
    let rows: Vec<(&str, &str)> = hits
        .iter()
        .map(|h| (h.file.as_str(), h.title.as_str()))
        .collect();
    assert!(rows.contains(&("glossary.md", "AIVAT")), "{rows:?}");
    assert!(rows.contains(&("k/eval.md", "Evaluation")), "{rows:?}");
    assert_eq!(rows.len(), 2);
}

#[test]
fn hits_report_terms_coverage_fields_related_and_updated() {
    let b = Brain::new();
    b.file("k/a.md", "---\nupdated: 2026-03-04\nrelated: [./b.md, ../missing.md, c.md, d.md, e.md]\n---\n# Rotate keys\n\n## How\n\nRotate the signing keys monthly.\n");
    for n in ["b", "c", "d", "e"] {
        b.file(&format!("k/{n}.md"), &neuron(n, "other"));
    }
    let hits = b.search("rotate signing keys yearly");
    let hit = &hits[0];
    assert_eq!(hit.file, "k/a.md");
    assert_eq!(hit.file_type, "markdown");
    assert_eq!(hit.title, "Rotate keys");
    assert_eq!(hit.matched_terms, ["rotate", "signing", "keys"]);
    assert!((hit.coverage - 0.75).abs() < 1e-9);
    assert_eq!(hit.matched_fields, ["title", "body"]);
    assert_eq!(hit.related, ["k/b.md", "k/c.md", "k/d.md"]);
    assert_eq!(hit.updated.as_deref(), Some("2026-03-04"));
    assert_eq!(hit.heading.as_deref(), Some("How"));
    assert!(hit.snippet.contains("Rotate the signing keys monthly."));
}

#[test]
fn matched_fields_include_headings_in_a_fixed_order() {
    let b = Brain::new();
    b.file(
        "k/n.md",
        "---\ntags: [zebra]\n---\n# Zebra\n\n## Zebra care\n\nfeed the zebra\n",
    );
    let hits = b.search("zebra");
    assert_eq!(hits[0].matched_fields, ["title", "tag", "heading", "body"]);
}

#[test]
fn scores_are_integers_and_results_are_totally_ordered() {
    let b = Brain::new();
    for name in ["b", "a", "c"] {
        b.file(
            &format!("k/{name}.md"),
            &neuron("Same", "identical zebra text"),
        );
    }
    let hits = b.search("zebra");
    assert_eq!(files(&hits), ["k/a.md", "k/b.md", "k/c.md"], "ties by file");
    assert!(hits.iter().all(|h| h.score == hits[0].score && h.score > 0));
}

// --- snippets ------------------------------------------------------------------------

#[test]
fn snippet_centres_on_the_window_with_most_matched_words() {
    let filler = "lorem ipsum dolor sit amet ".repeat(30);
    let text = format!("zebra once. {filler} zebra crossing zebra stripes here. {filler}");
    let spans: Vec<(usize, usize)> = text
        .match_indices("zebra")
        .map(|(i, m)| (i, i + m.len()))
        .collect();
    let s = snippet(&text, &spans);
    assert!(s.contains("zebra crossing zebra stripes"), "{s}");
    assert!(s.starts_with("...") && s.ends_with("..."), "{s}");
    assert!(s.chars().count() <= 200 + 6);
}

#[test]
fn snippets_never_split_a_character() {
    let text = format!("{}zebra{}", "ș".repeat(300), "日".repeat(300));
    let spans = vec![(600, 605)];
    let s = snippet(&text, &spans);
    assert!(s.contains("zebra"));
    assert!(
        s.trim_matches('.')
            .chars()
            .all(|c| c == 'ș' || c == '日' || "zebra".contains(c))
    );
    // Without matched words the snippet is the opening of the text.
    assert!(snippet(&text, &[]).starts_with("șșș"));
    assert_eq!(snippet("", &[]), "");
}

#[test]
fn search_writes_nothing() {
    let b = Brain::new();
    b.file("k/a.md", &neuron("A", "zebra"));
    let before = walk_names(&b.root);
    b.search("zebra");
    assert_eq!(walk_names(&b.root), before);
}

fn walk_names(root: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for e in fs::read_dir(&dir).unwrap() {
            let e = e.unwrap();
            if e.file_type().unwrap().is_dir() {
                stack.push(e.path());
            }
            out.push(e.path().display().to_string());
        }
    }
    out.sort();
    out
}

// --- T-79 review findings ----------------------------------------------------------

#[test]
fn malformed_neurons_are_skipped_and_reported() {
    // T-79 finding 2: `related: [` never closes, so the frontmatter does
    // not parse; the neuron is left out of the results and reported.
    use kluris_core::brain::store::ReadError;
    use kluris_core::frontmatter::Problem;
    let b = Brain::new();
    b.file("k/bad.md", "---\nrelated: [\n---\n# Bad\n\nzebra\n");
    b.file("k/good.md", &neuron("Good", "zebra"));
    b.file("glossary.md", "---\na: [\n---\n**Zebra** - an animal\n");
    let outcome = search(&b.store(), &SearchRequest::new("zebra"));
    assert_eq!(files(&outcome.hits), ["k/good.md"]);
    let skipped: Vec<&str> = outcome.skipped.iter().map(|s| s.rel.as_str()).collect();
    assert_eq!(skipped, ["k/bad.md", "glossary.md"]);
    for s in &outcome.skipped {
        assert!(
            matches!(&s.error, ReadError::File(e) if matches!(e.problem, Problem::Yaml(_))),
            "{s:?}"
        );
    }
    // Reported even when nothing is searched for.
    let mut req = SearchRequest::new("zebra");
    req.limit = 0;
    assert_eq!(search(&b.store(), &req).skipped.len(), 2);
    // A clean brain reports nothing.
    let clean = Brain::new();
    clean.file("k/good.md", &neuron("Good", "zebra"));
    assert!(
        search(&clean.store(), &SearchRequest::new("zebra"))
            .skipped
            .is_empty()
    );
}

/// `str.title()` of every code point Python 3.11 changes, and of strings that
/// exercise its context, from `parity/tools/gen_pycase.py` (the oracle).
#[test]
fn python_title_matches_the_oracle_for_every_code_point() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/py_title.tsv");
    let text = fs::read_to_string(path).unwrap();
    let decode = |hexes: &str| -> String {
        hexes
            .split(' ')
            .map(|h| char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap())
            .collect()
    };
    let mut checked = 0;
    let mut wrong = Vec::new();
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        let (input, output) = line.split_once('\t').unwrap();
        let (input, want) = (decode(input), decode(output));
        if py_title(&input) != want {
            wrong.push(format!("{input:?}: {:?} != {want:?}", py_title(&input)));
        }
        checked += 1;
    }
    assert!(checked > 1400, "{checked}");
    assert!(
        wrong.is_empty(),
        "{} wrong, first: {:?}",
        wrong.len(),
        &wrong[..wrong.len().min(10)]
    );
}

#[test]
fn python_title_context_from_the_review() {
    assert_eq!(py_title("ΟΣ"), "Ος");
    assert_eq!(py_title("თბილისი"), "თბილისი");
    assert_eq!(py_title("ᾈ"), "ᾈ");
}

#[test]
fn file_name_titles_use_python_title_case() {
    let b = Brain::new();
    b.file("k/ΟΣ.md", "---\nupdated: 2026-01-01\n---\nno heading\n");
    b.file("k/თბილისი.md", "plain body\n");
    b.file("k/ᾈ-x.yml", "#---\n# updated: 2026-01-01\n#---\nk: v\n");
    let mut titles: Vec<(String, String)> = collect_items(&b.store())
        .into_iter()
        .map(|i| (i.file, i.title))
        .collect();
    titles.sort();
    let want = [
        ("k/ΟΣ.md", "Ος"),
        ("k/თბილისი.md", "თბილისი"),
        ("k/ᾈ-x.yml", "ᾈ X"),
    ];
    let mut want: Vec<(String, String)> = want
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    want.sort();
    assert_eq!(titles, want);
}

#[test]
fn the_store_says_where_the_body_starts() {
    // T-79 finding 5: `line` comes from the store's read, not a second read.
    use kluris_core::frontmatter::{parse, parse_yaml_neuron};
    assert_eq!(parse("---\na: 1\n---\nbody\n").unwrap().body_line, 3);
    assert_eq!(
        parse("\u{feff}\n---\na: 1\n---\r\nbody\n")
            .unwrap()
            .body_line,
        4
    );
    assert_eq!(parse("---\r\na: 1\r\n---\r\nbody").unwrap().body_line, 3);
    assert_eq!(parse("# no frontmatter\n").unwrap().body_line, 0);
    assert_eq!(
        parse("---\na: 1\n").unwrap().body_line,
        0,
        "unclosed: no frontmatter"
    );
    assert_eq!(
        parse_yaml_neuron("\n#---\n# a: 1\n#---\nk: v\n")
            .unwrap()
            .body_line,
        4
    );
    assert_eq!(parse_yaml_neuron("k: v\n").unwrap().body_line, 0);
}

/// A store with no files behind it: search must read only through it.
struct MemStore {
    root: PathBuf,
    files: Vec<kluris_core::brain::walk::WalkedFile>,
    neurons: Vec<kluris_core::brain::neuron_index::Neuron>,
    texts: std::collections::BTreeMap<String, String>,
}

impl MemStore {
    fn new(texts: &[(&str, &str)]) -> MemStore {
        use kluris_core::brain::neuron_index::{Neuron, NeuronKind};
        use kluris_core::brain::walk::WalkedFile;
        let root = PathBuf::from("/nonexistent-kluris-mem-store");
        let texts: std::collections::BTreeMap<String, String> = texts
            .iter()
            .map(|(a, b)| (a.to_string(), b.to_string()))
            .collect();
        let files = texts
            .keys()
            .map(|rel| WalkedFile {
                rel: rel.clone(),
                path: root.join(rel),
                symlink: false,
            })
            .collect();
        let neurons = texts
            .keys()
            .filter(|rel| rel.ends_with(".md") && rel.contains('/'))
            .map(|rel| Neuron {
                rel: rel.clone(),
                path: root.join(rel),
                kind: NeuronKind::Markdown,
            })
            .collect();
        MemStore {
            root,
            files,
            neurons,
            texts,
        }
    }
}

impl kluris_core::brain::store::BrainStore for MemStore {
    fn root(&self) -> &Path {
        &self.root
    }
    fn files(&self) -> &[kluris_core::brain::walk::WalkedFile] {
        &self.files
    }
    fn neurons(&self) -> &[kluris_core::brain::neuron_index::Neuron] {
        &self.neurons
    }
    fn read(
        &self,
        rel: &str,
    ) -> Result<kluris_core::frontmatter::Frontmatter, kluris_core::brain::store::ReadError> {
        let text = self
            .texts
            .get(rel)
            .ok_or(kluris_core::brain::store::ReadError::Sandbox(
                kluris_core::brain::sandbox::SandboxError::NotFound,
            ))?;
        kluris_core::frontmatter::parse(text).map_err(|e| {
            kluris_core::brain::store::ReadError::File(kluris_core::frontmatter::FrontmatterError {
                path: self.root.join(rel),
                problem: kluris_core::frontmatter::Problem::Yaml(e),
            })
        })
    }
    fn lobe_names(&self) -> Vec<String> {
        vec!["k".into()]
    }
    fn lobe_neurons(&self, _lobe: &str) -> Vec<kluris_core::brain::neuron_index::Neuron> {
        self.neurons.clone()
    }
    fn structure_file(
        &self,
        rel: &str,
    ) -> Option<
        Result<kluris_core::frontmatter::Frontmatter, kluris_core::frontmatter::FrontmatterError>,
    > {
        self.texts.get(rel).map(|t| {
            kluris_core::frontmatter::parse(t).map_err(|e| {
                kluris_core::frontmatter::FrontmatterError {
                    path: self.root.join(rel),
                    problem: kluris_core::frontmatter::Problem::Yaml(e),
                }
            })
        })
    }
    fn structure_text(&self, rel: &str) -> Option<String> {
        self.texts.get(rel).cloned()
    }
}

#[test]
fn search_reads_only_through_the_store() {
    let store = MemStore::new(&[
        (
            "k/a.md",
            "---\nupdated: 2026-01-01\n---\n# A\n\n## Part\n\nzebra here\n",
        ),
        (
            "brain.md",
            "---\nupdated: 2026-01-01\n---\n\n# mem\n\n## Lobes\n\nzebra lobe\n",
        ),
    ]);
    let hits = search(&store, &SearchRequest::new("zebra")).hits;
    let lines: Vec<(&str, Option<usize>)> =
        hits.iter().map(|h| (h.file.as_str(), h.line)).collect();
    assert!(lines.contains(&("k/a.md", Some(6))), "{lines:?}");
    assert!(lines.contains(&("brain.md", Some(7))), "{lines:?}");
}

#[test]
fn every_query_term_is_tracked_past_64() {
    // T-79 P3: 64 absent words, then one that matches.
    let b = Brain::new();
    b.file("k/a.md", &neuron("A", "the needle is here"));
    let absent: Vec<String> = (0..64).map(|i| format!("absentword{i}x")).collect();
    let query = format!("{} needle", absent.join(" "));
    let hits = b.search(&query);
    assert_eq!(files(&hits), ["k/a.md"]);
    assert_eq!(hits[0].matched_terms, ["needle"]);
    assert_eq!(hits[0].coverage, 0.02);
}

#[cfg(unix)]
#[test]
fn a_backslash_in_a_posix_file_name_is_part_of_the_name() {
    // T-79 P3: `x\y.md` is one file on POSIX, read by its own name. v2
    // renders the path with `/` for `\` (`_rel`), so `file` is `k/x/y.md`,
    // and the title comes from the real stem.
    let b = Brain::new();
    b.file("k/x\\y.md", &neuron("Odd name", "zebra"));
    b.file(
        "k/a\\b.md",
        "---\nupdated: 2026-01-01\n---\nzebra without a heading\n",
    );
    let hits = b.search("zebra");
    let mut rows: Vec<(&str, &str, Option<usize>)> = hits
        .iter()
        .map(|h| (h.file.as_str(), h.title.as_str(), h.line))
        .collect();
    rows.sort();
    assert_eq!(
        rows,
        [
            ("k/a/b.md", "A\\B", Some(4)),
            ("k/x/y.md", "Odd name", Some(6))
        ]
    );
}

// --- T-84 finding 2: `related` through directory symlinks ---------------------------

#[cfg(unix)]
fn dir_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

#[cfg(windows)]
fn dir_symlink(target: &str, link: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_dir(target.replace('/', "\\"), link)
}

/// A brain with `k/alias -> real/deep`, `k/out -> ../../outside` (a
/// directory outside the brain) and `k/loop -> loop`; `None` where this
/// machine cannot make symlinks.
fn symlinked_brain(related: &str) -> Option<Brain> {
    let b = Brain::new();
    b.file(
        "k/a.md",
        &format!("---\nupdated: 2026-01-01\nrelated: {related}\n---\n# A\n\nquokka\n"),
    );
    b.file("k/real/deep/c.md", &neuron("C", "other"));
    b.file("k/real/b.md", &neuron("Real B", "other"));
    b.file("k/b.md", &neuron("B", "other"));
    let outside = b.root.parent().unwrap().join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("x.md"), neuron("X", "other")).unwrap();
    for (target, link) in [
        ("real/deep", "k/alias"),
        ("../../outside", "k/out"),
        ("loop", "k/loop"),
    ] {
        if let Err(e) = dir_symlink(target, &b.root.join(link)) {
            eprintln!("skipped: cannot make symlinks here ({e})");
            return None;
        }
    }
    Some(b)
}

/// Where `./alias/../b.md` leads. On POSIX the link is followed before
/// `..`, so `alias/..` is `k/real`. On Windows `..` is applied to the path
/// first (Win32 path parsing, and Python's `ntpath.realpath`, which
/// `sandbox::resolve` follows), so it is `k`, which is also the file Windows
/// opens for that path.
const ALIAS_PARENT_B: &str = if cfg!(windows) {
    "k/b.md"
} else {
    "k/real/b.md"
};

fn related_of(b: &Brain, file: &str) -> Vec<String> {
    let hits = b.search("quokka");
    hits.into_iter()
        .find(|h| h.file == file)
        .map(|h| h.related)
        .unwrap_or_default()
}

#[test]
fn related_links_follow_directory_symlinks() {
    let Some(b) = symlinked_brain("[./alias/c.md, ./alias/../b.md]") else {
        return;
    };
    assert_eq!(
        related_of(&b, "k/a.md"),
        ["k/real/deep/c.md", ALIAS_PARENT_B]
    );
}

#[test]
fn related_links_that_escape_or_loop_are_left_out() {
    let Some(b) =
        symlinked_brain("[./out/x.md, ../../outside/x.md, ./loop/y.md, /etc/passwd, ./b.md]")
    else {
        return;
    };
    assert_eq!(related_of(&b, "k/a.md"), ["k/b.md"]);
}

#[test]
fn the_local_store_resolves_links_on_disk() {
    use kluris_core::brain::store::BrainStore;
    let Some(b) = symlinked_brain("[]") else {
        return;
    };
    let store = b.store();
    assert_eq!(
        store.resolve_link("k/a.md", "./alias/c.md").as_deref(),
        Some("k/real/deep/c.md")
    );
    assert_eq!(
        store.resolve_link("k/a.md", "./alias/../b.md").as_deref(),
        Some(ALIAS_PARENT_B)
    );
    assert_eq!(
        store.resolve_link("k/a.md", "b.md").as_deref(),
        Some("k/b.md")
    );
    assert_eq!(store.resolve_link("k/a.md", "./out/x.md"), None);
    assert_eq!(store.resolve_link("k/a.md", "./loop/y.md"), None);
    assert_eq!(store.resolve_link("k/a.md", "../../outside/x.md"), None);
}

#[test]
fn a_store_without_files_resolves_links_by_path() {
    let store = MemStore::new(&[
        (
            "k/a.md",
            "---\nrelated: [./b.md, ../k/c.md, ../../x.md, ./b.md]\n---\n# A\n\nquokka\n",
        ),
        ("k/b.md", "# B\n\nother\n"),
        ("k/c.md", "# C\n\nother\n"),
    ]);
    let hits = search(&store, &SearchRequest::new("quokka")).hits;
    assert_eq!(hits[0].related, ["k/b.md", "k/c.md"]);
}
