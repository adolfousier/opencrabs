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
    fn rejects_pagers() {
        for cmd in ["less /etc/hosts", "more /var/log/syslog", "man bash"] {
            assert!(
                check_interactive_command(cmd).is_some(),
                "should reject: {cmd}"
            );
        }
    }

    #[test]
    fn pager_hint_suggests_cat() {
        let hint = check_interactive_command("less foo.log").expect("should reject");
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
    fn rejects_fzf_and_tmux() {
        assert!(check_interactive_command("fzf").is_some());
        assert!(check_interactive_command("tmux new-session").is_some());
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
