//! Channel Search & Message Capture Tests
//!
//! Tests for ChannelMessageRepository CRUD, multi-chat/multi-channel queries,
//! and the ChannelSearchTool agent operations.

// --- Repository Tests ---

mod repository {
    use crate::db::Database;
    use crate::db::models::ChannelMessage;
    use crate::db::repository::channel_message::ChannelMessageRepository;

    async fn setup() -> (Database, ChannelMessageRepository) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = ChannelMessageRepository::new(db.pool().clone());
        (db, repo)
    }

    fn msg(
        channel: &str,
        chat_id: &str,
        chat_name: &str,
        sender: &str,
        content: &str,
    ) -> ChannelMessage {
        ChannelMessage::new(
            channel.into(),
            chat_id.into(),
            Some(chat_name.into()),
            "user1".into(),
            sender.into(),
            content.into(),
            "text".into(),
            None,
        )
    }

    #[tokio::test]
    async fn test_insert_and_recent() {
        let (_db, repo) = setup().await;
        let m = msg("telegram", "-100111", "Group A", "Alice", "Hello world");
        repo.insert(&m).await.unwrap();

        let recent = repo.recent(Some("telegram"), "-100111", 10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].content, "Hello world");
        assert_eq!(recent[0].sender_name, "Alice");
        assert_eq!(recent[0].channel, "telegram");
    }

    #[tokio::test]
    async fn test_recent_respects_limit() {
        let (_db, repo) = setup().await;
        for i in 0..10 {
            let m = msg(
                "telegram",
                "-100111",
                "Group A",
                "Alice",
                &format!("msg {i}"),
            );
            repo.insert(&m).await.unwrap();
        }

        let recent = repo.recent(Some("telegram"), "-100111", 3).await.unwrap();
        assert_eq!(recent.len(), 3);
    }

    #[tokio::test]
    async fn test_recent_without_channel_filter() {
        let (_db, repo) = setup().await;
        repo.insert(&msg(
            "telegram",
            "-100111",
            "TG Group",
            "Alice",
            "from telegram",
        ))
        .await
        .unwrap();
        repo.insert(&msg(
            "discord",
            "-100111",
            "DC Group",
            "Bob",
            "from discord",
        ))
        .await
        .unwrap();

        // Same chat_id, no channel filter — both returned
        let recent = repo.recent(None, "-100111", 10).await.unwrap();
        assert_eq!(recent.len(), 2);
    }

    #[tokio::test]
    async fn test_recent_filters_by_channel() {
        let (_db, repo) = setup().await;
        repo.insert(&msg("telegram", "-100111", "TG Group", "Alice", "tg msg"))
            .await
            .unwrap();
        repo.insert(&msg("discord", "-100222", "DC Chan", "Bob", "dc msg"))
            .await
            .unwrap();

        let tg = repo.recent(Some("telegram"), "-100111", 10).await.unwrap();
        assert_eq!(tg.len(), 1);
        assert_eq!(tg[0].content, "tg msg");

        let dc = repo.recent(Some("discord"), "-100111", 10).await.unwrap();
        assert_eq!(dc.len(), 0);
    }

    #[tokio::test]
    async fn test_search_by_content() {
        let (_db, repo) = setup().await;
        repo.insert(&msg(
            "telegram",
            "-100111",
            "Group",
            "Alice",
            "the quick brown fox",
        ))
        .await
        .unwrap();
        repo.insert(&msg(
            "telegram",
            "-100111",
            "Group",
            "Bob",
            "lazy dog jumps",
        ))
        .await
        .unwrap();
        repo.insert(&msg("telegram", "-100111", "Group", "Carol", "hello world"))
            .await
            .unwrap();

        let results = repo
            .search(Some("telegram"), Some("-100111"), "fox", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].sender_name, "Alice");
    }

    #[tokio::test]
    async fn test_search_across_chats() {
        let (_db, repo) = setup().await;
        repo.insert(&msg(
            "telegram",
            "-100111",
            "Group A",
            "Alice",
            "deploy failed",
        ))
        .await
        .unwrap();
        repo.insert(&msg(
            "telegram",
            "-100222",
            "Group B",
            "Bob",
            "deploy succeeded",
        ))
        .await
        .unwrap();
        repo.insert(&msg(
            "slack",
            "C999",
            "General",
            "Carol",
            "deploy in progress",
        ))
        .await
        .unwrap();

        // Search all channels, all chats
        let results = repo.search(None, None, "deploy", 10).await.unwrap();
        assert_eq!(results.len(), 3);

        // Search telegram only, all chats
        let results = repo
            .search(Some("telegram"), None, "deploy", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 2);

        // Search specific chat only
        let results = repo
            .search(None, Some("-100111"), "deploy", 10)
            .await
            .unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_search_no_match() {
        let (_db, repo) = setup().await;
        repo.insert(&msg("telegram", "-100111", "Group", "Alice", "hello"))
            .await
            .unwrap();

        let results = repo
            .search(Some("telegram"), Some("-100111"), "nonexistent", 10)
            .await
            .unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_list_chats() {
        let (_db, repo) = setup().await;
        repo.insert(&msg("telegram", "-100111", "Group A", "Alice", "msg 1"))
            .await
            .unwrap();
        repo.insert(&msg("telegram", "-100111", "Group A", "Bob", "msg 2"))
            .await
            .unwrap();
        repo.insert(&msg("telegram", "-100222", "Group B", "Carol", "msg 3"))
            .await
            .unwrap();
        repo.insert(&msg("discord", "DC001", "Server Chan", "Dave", "msg 4"))
            .await
            .unwrap();

        // All channels
        let chats = repo.list_chats(None).await.unwrap();
        assert_eq!(chats.len(), 3);

        // Telegram only
        let chats = repo.list_chats(Some("telegram")).await.unwrap();
        assert_eq!(chats.len(), 2);

        // Find Group A — should have 2 messages
        let group_a = chats
            .iter()
            .find(|c| c.channel_chat_id == "-100111")
            .unwrap();
        assert_eq!(group_a.message_count, 2);
        assert_eq!(group_a.channel_chat_name.as_deref(), Some("Group A"));

        // Discord only
        let chats = repo.list_chats(Some("discord")).await.unwrap();
        assert_eq!(chats.len(), 1);
        assert_eq!(chats[0].message_count, 1);
    }

    #[tokio::test]
    async fn test_list_chats_empty() {
        let (_db, repo) = setup().await;
        let chats = repo.list_chats(None).await.unwrap();
        assert!(chats.is_empty());
    }

    #[tokio::test]
    async fn test_duplicate_insert_ignored() {
        let (_db, repo) = setup().await;
        let m = msg("telegram", "-100111", "Group", "Alice", "hello");
        repo.insert(&m).await.unwrap();
        // Same ID again — INSERT OR IGNORE
        repo.insert(&m).await.unwrap();

        let recent = repo.recent(Some("telegram"), "-100111", 10).await.unwrap();
        assert_eq!(recent.len(), 1);
    }

    #[tokio::test]
    async fn test_message_fields_roundtrip() {
        let (_db, repo) = setup().await;
        let m = ChannelMessage::new(
            "slack".into(),
            "C123".into(),
            Some("general".into()),
            "U456".into(),
            "Bob".into(),
            "test content".into(),
            "text".into(),
            Some("ts_789".into()),
        );
        let id = m.id;
        repo.insert(&m).await.unwrap();

        let recent = repo.recent(Some("slack"), "C123", 1).await.unwrap();
        assert_eq!(recent.len(), 1);
        let r = &recent[0];
        assert_eq!(r.id, id);
        assert_eq!(r.channel, "slack");
        assert_eq!(r.channel_chat_id, "C123");
        assert_eq!(r.channel_chat_name.as_deref(), Some("general"));
        assert_eq!(r.sender_id, "U456");
        assert_eq!(r.sender_name, "Bob");
        assert_eq!(r.content, "test content");
        assert_eq!(r.message_type, "text");
        assert_eq!(r.platform_message_id.as_deref(), Some("ts_789"));
    }
}

// --- ChannelSearchTool Tests ---

mod tool {
    use crate::brain::tools::channel_search::ChannelSearchTool;
    use crate::brain::tools::{Tool, ToolExecutionContext};
    use crate::db::Database;
    use crate::db::models::ChannelMessage;
    use crate::db::repository::channel_message::ChannelMessageRepository;

    async fn setup() -> (Database, ChannelMessageRepository, ChannelSearchTool) {
        let db = Database::connect_in_memory()
            .await
            .expect("Failed to create database");
        db.run_migrations().await.expect("Failed to run migrations");
        let repo = ChannelMessageRepository::new(db.pool().clone());
        let tool = ChannelSearchTool::new(repo.clone());
        (db, repo, tool)
    }

    fn ctx() -> ToolExecutionContext {
        ToolExecutionContext::new(uuid::Uuid::new_v4())
    }

    fn insert_msg(
        channel: &str,
        chat_id: &str,
        chat_name: &str,
        sender: &str,
        content: &str,
    ) -> ChannelMessage {
        ChannelMessage::new(
            channel.into(),
            chat_id.into(),
            Some(chat_name.into()),
            "u1".into(),
            sender.into(),
            content.into(),
            "text".into(),
            None,
        )
    }

    #[test]
    fn test_tool_name_and_schema() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let (_db, _repo, tool) = setup().await;
            assert_eq!(tool.name(), "channel_search");
            let schema = tool.input_schema();
            let props = schema["properties"].as_object().unwrap();
            assert!(props.contains_key("operation"));
            assert!(props.contains_key("channel"));
            assert!(props.contains_key("chat_id"));
            assert!(props.contains_key("query"));
            assert!(props.contains_key("n"));
            assert!(!tool.requires_approval());
        });
    }

    #[tokio::test]
    async fn test_list_chats_empty() {
        let (_db, _repo, tool) = setup().await;
        let input = serde_json::json!({"operation": "list_chats"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No channel messages captured"));
    }

    #[tokio::test]
    async fn test_list_chats_with_data() {
        let (_db, repo, tool) = setup().await;
        let m1 = insert_msg("telegram", "-100111", "Dev Group", "Alice", "hello");
        let m2 = insert_msg("telegram", "-100222", "Ops Group", "Bob", "world");
        repo.insert(&m1).await.unwrap();
        repo.insert(&m2).await.unwrap();

        let input = serde_json::json!({"operation": "list_chats"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Known chats (2)"));
        assert!(result.output.contains("Dev Group"));
        assert!(result.output.contains("Ops Group"));
    }

    #[tokio::test]
    async fn test_list_chats_filtered_by_channel() {
        let (_db, repo, tool) = setup().await;
        let m1 = insert_msg("telegram", "-100111", "TG Group", "Alice", "tg");
        let m2 = insert_msg("discord", "DC001", "DC Chan", "Bob", "dc");
        repo.insert(&m1).await.unwrap();
        repo.insert(&m2).await.unwrap();

        let input = serde_json::json!({"operation": "list_chats", "channel": "telegram"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Known chats (1)"));
        assert!(result.output.contains("TG Group"));
        assert!(!result.output.contains("DC Chan"));
    }

    #[tokio::test]
    async fn test_recent_requires_chat_id() {
        let (_db, _repo, tool) = setup().await;
        let input = serde_json::json!({"operation": "recent"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        let msg = result.error.as_deref().unwrap_or(&result.output);
        assert!(msg.contains("chat_id"));
    }

    #[tokio::test]
    async fn test_recent_returns_messages() {
        let (_db, repo, tool) = setup().await;
        let m1 = insert_msg("telegram", "-100111", "Group", "Alice", "first message");
        let m2 = insert_msg("telegram", "-100111", "Group", "Bob", "second message");
        repo.insert(&m1).await.unwrap();
        repo.insert(&m2).await.unwrap();

        let input = serde_json::json!({"operation": "recent", "chat_id": "-100111"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("first message"));
        assert!(result.output.contains("second message"));
        assert!(result.output.contains("Alice"));
        assert!(result.output.contains("Bob"));
    }

    #[tokio::test]
    async fn test_recent_empty_chat() {
        let (_db, _repo, tool) = setup().await;
        let input = serde_json::json!({"operation": "recent", "chat_id": "-999"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No messages found"));
    }

    #[tokio::test]
    async fn test_recent_with_n_limit() {
        let (_db, repo, tool) = setup().await;
        for i in 0..10 {
            let m = insert_msg("telegram", "-100111", "Group", "Alice", &format!("msg {i}"));
            repo.insert(&m).await.unwrap();
        }

        let input = serde_json::json!({"operation": "recent", "chat_id": "-100111", "n": 3});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("(3)"));
    }

    #[tokio::test]
    async fn test_search_requires_query() {
        let (_db, _repo, tool) = setup().await;
        let input = serde_json::json!({"operation": "search"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        let msg = result.error.as_deref().unwrap_or(&result.output);
        assert!(msg.contains("query"));
    }

    #[tokio::test]
    async fn test_search_finds_messages() {
        let (_db, repo, tool) = setup().await;
        let m1 = insert_msg(
            "telegram",
            "-100111",
            "Group",
            "Alice",
            "deploy failed on prod",
        );
        let m2 = insert_msg("telegram", "-100111", "Group", "Bob", "checking logs now");
        let m3 = insert_msg(
            "slack",
            "C999",
            "General",
            "Carol",
            "deploy succeeded on staging",
        );
        repo.insert(&m1).await.unwrap();
        repo.insert(&m2).await.unwrap();
        repo.insert(&m3).await.unwrap();

        let input = serde_json::json!({"operation": "search", "query": "deploy"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("(2)")); // 2 results
        assert!(result.output.contains("Alice"));
        assert!(result.output.contains("Carol"));
    }

    #[tokio::test]
    async fn test_search_with_channel_filter() {
        let (_db, repo, tool) = setup().await;
        let m1 = insert_msg("telegram", "-100111", "Group", "Alice", "error happened");
        let m2 = insert_msg("slack", "C999", "General", "Bob", "error resolved");
        repo.insert(&m1).await.unwrap();
        repo.insert(&m2).await.unwrap();

        let input =
            serde_json::json!({"operation": "search", "query": "error", "channel": "telegram"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("(1)"));
        assert!(result.output.contains("Alice"));
        assert!(!result.output.contains("Bob"));
    }

    #[tokio::test]
    async fn test_search_no_match() {
        let (_db, repo, tool) = setup().await;
        let m = insert_msg("telegram", "-100111", "Group", "Alice", "hello");
        repo.insert(&m).await.unwrap();

        let input = serde_json::json!({"operation": "search", "query": "nonexistent"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No messages matching"));
    }

    #[tokio::test]
    async fn test_unknown_operation() {
        let (_db, _repo, tool) = setup().await;
        let input = serde_json::json!({"operation": "invalid"});
        let result = tool.execute(input, &ctx()).await.unwrap();
        assert!(!result.success);
        let msg = result.error.as_deref().unwrap_or(&result.output);
        assert!(msg.contains("Unknown operation"));
    }
}
