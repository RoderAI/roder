use super::*;

impl BoardAction {
    pub(crate) fn is_read_only(&self) -> bool {
        matches!(
            self,
            Self::ListChannels | Self::ListPosts { .. } | Self::ReadPost { .. }
        )
    }

    pub(crate) fn read(&self, team: Option<&TeamState>, caller: &str) -> anyhow::Result<Value> {
        let empty = TeamMessageBoard::default();
        let (board, actor) = match team {
            None => (&empty, ""),
            Some(team) => {
                let actor = team
                    .members
                    .iter()
                    .find(|member| {
                        member.thread_id == caller && member.status != TeamMemberStatus::Closed
                    })
                    .ok_or_else(|| {
                        anyhow::anyhow!("caller is not an active member of this team")
                    })?;
                (&team.board, actor.id.as_str())
            }
        };
        match self {
            Self::ListChannels => Ok(board.0.channels_for(actor)),
            Self::ListPosts {
                channel,
                query,
                before,
                limit,
            } => board
                .0
                .list_posts(channel.as_deref(), query.as_deref(), *before, *limit),
            Self::ReadPost { post_id } => board.0.post(*post_id).map(|post| json!(post)),
            _ => anyhow::bail!("expected a read-only board action"),
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
        let query = query
            .map(|value| {
                anyhow::ensure!(
                    value.len() <= MAX_TEXT_BYTES,
                    "board query exceeds {MAX_TEXT_BYTES} bytes"
                );
                Ok::<_, anyhow::Error>(
                    regex::RegexBuilder::new(&regex::escape(value))
                        .case_insensitive(true)
                        .build()?,
                )
            })
            .transpose()?;
        let mut matches = self.posts[..end].iter().rev().filter(|post| {
            channel.is_none_or(|channel| post.channel == channel)
                && query
                    .as_ref()
                    .is_none_or(|query| query.is_match(&post.text))
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
