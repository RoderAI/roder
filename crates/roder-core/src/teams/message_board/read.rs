use super::*;

impl BoardAction {
    pub(crate) fn read_empty_board(&self) -> Option<anyhow::Result<Value>> {
        self.read(&TeamMessageBoard::default(), "")
    }

    pub(super) fn read(
        &self,
        board: &TeamMessageBoard,
        actor: &str,
    ) -> Option<anyhow::Result<Value>> {
        match self {
            Self::ListChannels => Some(Ok(board.0.channels_for(actor))),
            Self::ListPosts {
                channel,
                query,
                before,
                limit,
            } => Some(
                board
                    .0
                    .list_posts(channel.as_deref(), query.as_deref(), *before, *limit),
            ),
            Self::ReadPost { post_id } => Some(board.0.post(*post_id).map(|post| json!(post))),
            _ => None,
        }
    }
}

impl BoardState {
    fn list_posts(
        &self,
        channel: Option<&str>,
        query: Option<&str>,
        before: Option<Uuid>,
        limit: Option<usize>,
    ) -> anyhow::Result<Value> {
        if let Some(channel) = channel {
            anyhow::ensure!(
                self.contains_channel(channel),
                "unknown channel {channel:?}"
            );
        }
        let limit = limit.unwrap_or(10);
        anyhow::ensure!(
            (1..=MAX_PAGE_SIZE).contains(&limit),
            "limit must be between 1 and {MAX_PAGE_SIZE}"
        );
        let end = match before {
            Some(id) => self
                .posts
                .iter()
                .position(|post| post.id == id)
                .ok_or_else(|| anyhow::anyhow!("unknown board cursor {id}"))?,
            None => self.posts.len(),
        };
        let query = query.map(|value| value.to_lowercase());
        let mut matches = self.posts[..end].iter().rev().filter(|post| {
            channel.is_none_or(|channel| post.channel == channel)
                && query
                    .as_ref()
                    .is_none_or(|query| post.text.to_lowercase().contains(query))
        });
        let mut posts = Vec::new();
        let mut last = None;
        for post in matches.by_ref().take(limit) {
            let work = match &post.work {
                None => Value::Null,
                Some(BoardWork::Completed { owner, .. }) => {
                    json!({ "status": "completed", "owner": owner })
                }
                work => json!(work),
            };
            posts.push(json!({
                "id": post.id,
                "channel": post.channel,
                "author": post.author,
                "reply_to": post.reply_to,
                "work": work,
                "text_preview": post.text.chars().take(160)
                    .map(|ch| if ch.is_control() { ' ' } else { ch })
                    .collect::<String>(),
            }));
            last = Some(post.id);
        }
        let next_before = matches.next().and(last);
        Ok(json!({ "posts": posts, "next_before": next_before }))
    }
}
