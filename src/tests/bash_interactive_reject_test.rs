//! Tests for `check_interactive_command` — the bash-tool pre-flight that
//! refuses to run commands needing a real TTY.
//!
//! Background: 2026-04-23 we set bash subprocess stdin to /dev/null
//! (commit 195f56e) so mouse-mode escapes couldn't bleed into tool
//! output. Side effect: `git add -p` and friends now exit silently
//! with code 0 on EOF. The agent reads the (still-printed) prompt
//! text, decides "this would hang", explains to the user, then
//! retries the same command — a free self-loop. This filter cuts
//! the loop on attempt 1 by surfacing a clear non-interactive
//! alternative.

use crate::brain::tools::bash::check_interactive_command;

mod git {
    use super::*;

    #[test]
    fn rejects_git_add_p() {
        // The exact form the user hit on 2026-04-26.
        let hint = check_interactive_command("git add -p lib/presentation/deal_room_screen.dart")
            .expect("should reject");
        assert!(hint.contains("git add -p"));
        assert!(hint.contains("git add <path>") || hint.contains("git add -A"));
    }

    #[test]
    fn rejects_git_add_patch_long_form() {
        assert!(check_interactive_command("git add --patch foo.txt").is_some());
    }

    #[test]
    fn rejects_git_add_i() {
        assert!(check_interactive_command("git add -i").is_some());
    }

    #[test]
    fn rejects_git_add_interactive_long_form() {
        assert!(check_interactive_command("git add --interactive").is_some());
    }

    #[test]
    fn allows_plain_git_add() {
        // The non-interactive happy path must NOT fire — it's how the
        // agent should be staging files.
        assert!(check_interactive_command("git add lib/foo.dart").is_none());
        assert!(check_interactive_command("git add -A").is_none());
        assert!(check_interactive_command("git add .").is_none());
    }

    #[test]
    fn rejects_git_rebase_interactive() {
        assert!(check_interactive_command("git rebase -i HEAD~3").is_some());
        assert!(check_interactive_command("git rebase --interactive main").is_some());
    }

    #[test]
    fn allows_plain_git_rebase() {
        assert!(check_interactive_command("git rebase main").is_none());
        assert!(check_interactive_command("git rebase --abort").is_none());
    }

    #[test]
    fn rejects_git_commit_no_message() {
        // Bare `git commit` opens the editor.
        let hint = check_interactive_command("git commit").expect("should reject");
        assert!(hint.contains("editor"));
    }

    #[test]
    fn allows_git_commit_with_dash_m() {
        assert!(check_interactive_command("git commit -m \"fix typo\"").is_none());
        assert!(check_interactive_command("git commit -am \"fix typo\"").is_none());
    }

    #[test]
    fn allows_git_commit_with_message_long_form() {
        assert!(check_interactive_command("git commit --message=\"foo\"").is_none());
    }

    #[test]
    fn allows_git_commit_amend_no_edit() {
        assert!(check_interactive_command("git commit --amend --no-edit").is_none());
    }

    #[test]
    fn allows_git_commit_with_file() {
        assert!(check_interactive_command("git commit -F /tmp/msg.txt").is_none());
        assert!(check_interactive_command("git commit --file=/tmp/msg.txt").is_none());
    }
}

mod editors {
    use super::*;

    #[test]
    fn rejects_vim_and_friends() {
        for cmd in [
            "vim foo.txt",
            "vi bar",
            "nvim baz",
            "nano qux",
            "emacs x",
            "pico y",
        ] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject editor: {cmd}"
            );
        }
    }

    #[test]
    fn editor_hint_mentions_edit_file_alternative() {
        let hint = check_interactive_command("vim foo.txt").expect("should reject");
        assert!(hint.contains("edit_file") || hint.contains("write_file"));
    }
}

mod pagers_and_tuis {
    use super::*;

    #[test]
    fn rejects_operandless_pagers() {
        // #555: this arm now tests whether the pager has anything to page,
        // instead of firing on the bare name. Measured with stdin=/dev/null and
        // stdout a pipe: bare `less`, `less -R` and bare `more` all return rc=0
        // with ZERO bytes of output — the silent no-op this guard breaks.
        // `less -h` is in this list deliberately: it also prints nothing (see
        // `has_info_flag`), so accepting it would be a regression.
        for cmd in ["less", "more", "man", "less -R", "man -k", "less -h"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject operand-less pager: {cmd}"
            );
        }
    }

    #[test]
    fn allows_pagers_with_an_operand() {
        // #555 regression: every one of these was rejected before, and each was
        // measured working here — `less /etc/hosts` → 10 bytes,
        // `man man` → 36948 bytes, `man -w python3` → 36 bytes.
        for cmd in [
            "less /etc/hosts",
            "more /var/log/syslog",
            "man bash",
            "man -w python3",
            "man -k printf",
            "less -R /etc/hosts",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow pager with an operand: {cmd}"
            );
        }
    }

    #[test]
    fn allows_pager_help_and_version() {
        // Operand-less but self-sufficient: measured `less --version` → 285
        // bytes and `less --help` → 13067. LONG forms only — `less -h` prints
        // nothing and stays rejected above.
        assert!(check_interactive_command("less --version").is_none());
        assert!(check_interactive_command("less --help").is_none());
    }

    #[test]
    fn pager_hint_suggests_cat() {
        let hint = check_interactive_command("less").expect("should reject");
        assert!(hint.contains("cat"));
    }

    #[test]
    fn rejects_top_family() {
        for cmd in ["top", "htop", "btop"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject: {cmd}"
            );
        }
    }

    #[test]
    fn top_hint_suggests_ps() {
        let hint = check_interactive_command("top").expect("should reject");
        assert!(hint.contains("ps"));
    }

    #[test]
    fn rejects_bare_tui_tools() {
        // #555: only the argument-less form is the trap. Measured, bare `tmux`
        // and bare `screen` fail immediately without a TTY.
        for cmd in ["fzf", "gum", "tmux", "screen"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject bare TUI tool: {cmd}"
            );
        }
    }

    #[test]
    fn allows_tui_tools_with_arguments() {
        // #555 regression: `tmux new-session` was rejected before. Measured on a
        // private socket, both with and without a server already running: it
        // exits rc=1 with "open terminal failed: not a terminal" and creates NO
        // session (the session count stayed at 1), so it is a loud failure, not
        // the silent no-op this guard targets. `tmux -V` / `screen --version`
        // print real output.
        for cmd in [
            "tmux -V",
            "tmux ls",
            "screen -ls",
            "screen --version",
            "tmux new-session",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow TUI tool with arguments: {cmd}"
            );
        }
    }
}

mod repls {
    use super::*;

    #[test]
    fn rejects_bare_python() {
        assert!(check_interactive_command("python").is_some());
        assert!(check_interactive_command("python3").is_some());
    }

    #[test]
    fn allows_python_with_dash_c() {
        assert!(check_interactive_command("python -c \"print('hi')\"").is_none());
        assert!(check_interactive_command("python3 -c \"print('hi')\"").is_none());
    }

    #[test]
    fn allows_python_with_script() {
        // `python script.py` — script arg, not a flag.
        assert!(check_interactive_command("python script.py").is_none());
    }

    #[test]
    fn allows_python_with_dash_m() {
        // `python -m <module>` always runs non-interactively, never opens a REPL.
        assert!(check_interactive_command("python -m pytest").is_none());
        assert!(check_interactive_command("python3 -m pytest").is_none());
        assert!(check_interactive_command("python -m pip install foo").is_none());
        assert!(check_interactive_command("python -m http.server").is_none());
        assert!(check_interactive_command("python -m json.tool").is_none());
    }

    #[test]
    fn rejects_bare_node() {
        assert!(check_interactive_command("node").is_some());
    }

    #[test]
    fn allows_node_with_eval() {
        assert!(check_interactive_command("node -e \"console.log(1)\"").is_none());
    }
}

mod database_clis {
    use super::*;

    #[test]
    fn rejects_psql_without_command_or_file() {
        assert!(check_interactive_command("psql -h localhost mydb").is_some());
    }

    #[test]
    fn allows_psql_with_dash_c() {
        assert!(check_interactive_command("psql -c \"SELECT 1\"").is_none());
    }

    #[test]
    fn allows_psql_with_dash_f() {
        assert!(check_interactive_command("psql -f script.sql").is_none());
    }

    #[test]
    fn rejects_mysql_without_dash_e() {
        assert!(check_interactive_command("mysql -u root -p").is_some());
    }

    #[test]
    fn allows_mysql_with_dash_e() {
        assert!(check_interactive_command("mysql -u root -e \"SHOW TABLES\"").is_none());
    }

    #[test]
    fn rejects_bare_redis_cli() {
        assert!(check_interactive_command("redis-cli").is_some());
    }

    #[test]
    fn allows_redis_cli_with_command() {
        assert!(check_interactive_command("redis-cli GET mykey").is_none());
    }
}

mod chained_commands {
    use super::*;

    #[test]
    fn detects_interactive_in_second_segment_of_chain() {
        // The 2026-04-26 case: `cd ~/srv/dart/myapp && git add -p ...`.
        // The chain prefix is fine but the segment after `&&` is not.
        let cmd = "cd ~/srv/dart/myapp && git add -p lib/foo.dart";
        assert!(check_interactive_command(cmd).is_some());
    }

    #[test]
    fn detects_editor_in_pipeline() {
        let cmd = "echo hi | vim -";
        assert!(check_interactive_command(cmd).is_some());
    }

    #[test]
    fn detects_after_semicolon() {
        let cmd = "ls; htop";
        assert!(check_interactive_command(cmd).is_some());
    }

    #[test]
    fn allows_chain_of_non_interactive() {
        let cmd = "cd /tmp && git status && cat file.txt";
        assert!(check_interactive_command(cmd).is_none());
    }
}

mod normal_commands {
    use super::*;

    #[test]
    fn does_not_false_fire_on_common_commands() {
        for cmd in [
            "ls -la",
            "cat README.md",
            "grep -r foo src/",
            "cargo build --release",
            "npm install",
            "echo hello",
            "git status",
            "git diff --stat",
            "git log --oneline -10",
            "git push origin main",
            "make test",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "false positive on: {cmd}"
            );
        }
    }
}

mod quote_awareness {
    use super::*;

    #[test]
    fn allows_pipes_and_alternation_inside_double_quotes() {
        // Issue #205: pipe or semicolon inside double-quoted string or grep regex
        // should not be treated as a pipeline or command separator.
        assert!(check_interactive_command("git log --grep=\"feat: add | fix: remove\"").is_none());
        assert!(check_interactive_command("echo \"git push | git commit\"").is_none());
        assert!(check_interactive_command("git commit -m \"fix: foo; bar | baz & qux\"").is_none());
    }

    #[test]
    fn allows_pipes_and_semicolons_inside_single_quotes() {
        assert!(check_interactive_command("git log --grep='feat: add | fix: remove'").is_none());
        assert!(check_interactive_command("echo 'git commit'").is_none());
        assert!(check_interactive_command("echo 'foo | git commit'").is_none());
    }

    #[test]
    fn allows_escaped_pipes_and_semicolons() {
        assert!(check_interactive_command("echo foo \\| git commit").is_none());
        assert!(check_interactive_command("echo foo \\; vim").is_none());
    }

    #[test]
    fn still_detects_real_interactive_after_quoted_arg() {
        // Pipeline outside quotes must still be split and checked
        let cmd = "echo \"hello world\" | vim -";
        assert!(check_interactive_command(cmd).is_some());

        let cmd2 = "echo 'my commit message' | git commit";
        assert!(check_interactive_command(cmd2).is_some());

        let cmd3 = "git commit -m \"foo\"; nano bar.txt";
        assert!(check_interactive_command(cmd3).is_some());
    }
}

mod issue_555_non_interactive_one_shots {
    use super::*;

    #[test]
    fn allows_repl_informational_and_one_shot_flags() {
        // The headline defect: the arm tested a 3-string allowlist (-c/-e/-m),
        // so every other non-interactive flag was refused. Each of these was
        // measured running to completion with rc=0 on this host — `node
        // --version` → "v22.22.3", `python3 --version` → "Python 3.14.6",
        // `python3 --help` → usage.
        for cmd in [
            "node --version",
            "node -v",
            "node --help",
            "node -p 1+1",
            "node -e console.log(1)",
            "node --check /tmp/x.js",
            "node -i",
            "python --version",
            "python3 --version",
            "python3 --help",
            "python3 -m pytest",
            "python3 -c print(1)",
            "python3 script.py",
            "node --interactive",
        ] {
            let verdict = check_interactive_command(cmd);
            assert_eq!(
                verdict.is_some(),
                matches!(cmd, "node -i" | "node --interactive"),
                "unexpected verdict for {cmd}: {verdict:?}"
            );
        }
    }

    #[test]
    fn still_rejects_bare_repls() {
        // The protection the arm exists for: bare invocations read /dev/null
        // stdin, exit 0 and print NOTHING — the silent no-op that stalls the
        // loop. Measured: bare `node` and `python3` → rc=0, 0 bytes.
        for cmd in ["node", "python", "python3", "irb", "ghci", "scala"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should still reject bare REPL: {cmd}"
            );
        }
    }

    #[test]
    fn repl_hint_states_the_real_reason() {
        // The old message claimed the commands "hang on /dev/null stdin". They
        // do not — they exit 0 immediately. The message must not carry the
        // unreproducible claim.
        let hint = check_interactive_command("node").expect("should reject");
        assert!(!hint.to_lowercase().contains("hang"), "gone: {hint}");
    }

    #[test]
    fn allows_editor_info_flags_but_not_a_file_operand() {
        // Measured: `vim --version` → 3440 bytes, `vim -h` → 2369,
        // `nano -h` → 4365, `ed -h` → 2022 — all real output.
        for cmd in [
            "vim --version",
            "vim -h",
            "nano -h",
            "ed -h",
            "pico -h",
            "nano --version",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow editor info flag: {cmd}"
            );
        }
        // But editing a FILE needs a TTY: measured `vim FILE` → rc=1 with
        // escape-sequence noise, `nano FILE` → "standard input is not a
        // terminal".
        for cmd in ["vim /etc/hostname", "nano /etc/hostname", "ed /etc/hostname"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject editor on a file: {cmd}"
            );
        }
    }

    #[test]
    fn allows_top_family_batch_flags() {
        // Measured: bare `top`/`htop` fail immediately (rc=1), while
        // `top -b -n 1` → 13234 bytes and `htop --version` → 11 bytes.
        assert!(check_interactive_command("top -b -n 1").is_none());
        assert!(check_interactive_command("htop --version").is_none());
        assert!(check_interactive_command("top -h").is_none());
        assert!(check_interactive_command("top").is_some());
        assert!(check_interactive_command("htop").is_some());
    }
}

mod issue_555_sql_client_flag_spellings {
    use super::*;

    #[test]
    fn allows_psql_command_in_every_spelling() {
        // The old test was `contains(" -c ")` — a space-delimited SUBSTRING, so
        // it rejected the joined form and both long forms even though they are
        // exactly as non-interactive.
        for cmd in [
            "psql -c \"SELECT 1\"",
            "psql -c\"SELECT 1\"",
            "psql --command=\"SELECT 1\"",
            "psql --command \"SELECT 1\"",
            "psql -f script.sql",
            "psql --file=script.sql",
            "psql -c\"SELECT 1\" mydb",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow psql with a command source: {cmd}"
            );
        }
    }

    #[test]
    fn still_rejects_psql_without_a_command_source() {
        assert!(check_interactive_command("psql").is_some());
        assert!(check_interactive_command("psql mydb").is_some());
        assert!(check_interactive_command("psql -h localhost mydb").is_some());
    }

    #[test]
    fn allows_mysql_execute_in_every_spelling() {
        for cmd in [
            "mysql -e \"SHOW TABLES\"",
            "mysql --execute=\"SHOW TABLES\"",
            "mysql -u root -e \"SHOW TABLES\"",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow mysql with a statement: {cmd}"
            );
        }
        assert!(check_interactive_command("mysql -u root -p").is_some());
    }

    #[test]
    fn rejects_redis_cli_that_would_open_a_repl() {
        // The old test was `word count == 1`, so it let the arm's OWN failure
        // mode through: `redis-cli -h HOST` has two words and opens a REPL.
        // NOTE: redis-cli is not installed on this host, so unlike the rest of
        // this file this case is derived from the documented CLI contract
        // rather than measured.
        for cmd in [
            "redis-cli",
            "redis-cli -h localhost",
            "redis-cli --host=localhost",
            "redis-cli -p 6379",
            "redis-cli -u redis://localhost -n 3",
        ] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject redis-cli without a subcommand: {cmd}"
            );
        }
    }

    #[test]
    fn allows_redis_cli_with_a_subcommand() {
        for cmd in [
            "redis-cli GET mykey",
            "redis-cli -h localhost GET mykey",
            "redis-cli -p 6379 PING",
            "redis-cli --scan",
        ] {
            assert!(
                check_interactive_command(cmd).is_none(),
                "should allow redis-cli with a subcommand: {cmd}"
            );
        }
    }
}

mod issue_555_advisory_not_a_boundary {
    use super::*;

    #[test]
    fn absolute_path_is_not_matched_by_design() {
        // #555 finding: the guard compares the literal first token, so an
        // absolute path is NOT intercepted. This is documented behaviour, not a
        // safety assertion — the description now says so explicitly. Measured:
        // `/usr/local/bin/node --version` → "v22.22.3" rc=0.
        assert!(check_interactive_command("/usr/local/bin/node --version").is_none());
        assert!(check_interactive_command("/usr/bin/less --version").is_none());
    }

    #[test]
    fn detects_a_guarded_word_inside_a_multi_line_command() {
        // Measured: `\n` is a segment separator, so a single bare pager or REPL
        // anywhere in a multi-statement command vetoes the WHOLE call — the
        // other lines never run. Pinned here so the behaviour is explicit.
        let cmd = "echo one\nless\necho three";
        assert!(check_interactive_command(cmd).is_some());
    }
}
