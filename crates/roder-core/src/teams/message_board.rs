use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use roder_api::teams::{TeamMemberId, TeamMemberStatus};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::OffsetDateTime;
use uuid::Uuid;

use super::{TeamManager, TeamState};

#[path = "message_board/read.rs"]
mod read;

const MAX_CHANNELS: usize = 32;
const MAX_POSTS: usize = 4096;
const MAX_TEXT_BYTES: usize = 8192;
const MAX_DESCRIPTION_BYTES: usize = 512;
const MAX_PAGE_SIZE: usize = 25;

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct TeamMessageBoard(Arc<BoardState>);

impl TeamMessageBoard {
    pub fn is_empty(&self) -> bool {
        self.0.channels.is_empty() && self.0.posts.is_empty()
    }

    fn edit(&mut self) -> &mut BoardState {
        Arc::make_mut(&mut self.0)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct BoardState {
    channels: BTreeMap<String, BoardChannel>,
    posts: Vec<BoardPost>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
struct BoardChannel {
    description: String,
    subscribers: BTreeSet<TeamMemberId>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct BoardPost {
    id: Uuid,
    channel: String,
    author: TeamMemberId,
    text: String,
    reply_to: Option<Uuid>,
    work: Option<BoardWork>,
    #[serde(with = "time::serde::rfc3339")]
    created_at: OffsetDateTime,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "status", rename_all = "snake_case")]
enum BoardWork {
    Open,
    Claimed {
        owner: TeamMemberId,
    },
    Completed {
        owner: TeamMemberId,
        evidence: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum BoardAction {
    ListChannels,
    CreateChannel {
        channel: String,
        #[serde(default)]
        description: String,
    },
    Subscribe {
        channel: String,
    },
    Unsubscribe {
        channel: String,
    },
    Post {
        #[serde(default = "general_channel")]
        channel: String,
        text: String,
        #[serde(default)]
        claimable: bool,
        reply_to: Option<Uuid>,
        #[serde(default)]
        notify: Vec<String>,
    },
    ListPosts {
        channel: Option<String>,
        query: Option<String>,
        before: Option<Uuid>,
        limit: Option<usize>,
    },
    ReadPost {
        post_id: Uuid,
    },
    Claim {
        post_id: Uuid,
    },
    Release {
        post_id: Uuid,
    },
    Handoff {
        post_id: Uuid,
        target: String,
    },
    Complete {
        post_id: Uuid,
        evidence: String,
    },
}

pub(crate) struct BoardResult {
    pub value: Value,
    pub recipients: BTreeSet<TeamMemberId>,
}

impl TeamManager {
    pub(crate) async fn apply_board_action(
        &self,
        team_id: &str,
        actor: &str,
        action: BoardAction,
    ) -> anyhow::Result<BoardResult> {
        let mut teams = self.teams.write().await;
        let mut team = self
            .load_locked(&mut teams, team_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("unknown team {team_id:?}"))?;
        ensure_active_member(&team, actor)?;
        let mut recipients = BTreeSet::new();
        if let Some(result) = action.read(&team.board, actor) {
            return Ok(BoardResult {
                value: result?,
                recipients,
            });
        }
        let value = match action {
            BoardAction::ListChannels
            | BoardAction::ListPosts { .. }
            | BoardAction::ReadPost { .. } => {
                unreachable!("read-only board actions return before mutation")
            }
            BoardAction::CreateChannel {
                channel,
                description,
            } => {
                validate_channel(&channel)?;
                anyhow::ensure!(
                    encoded_text_fits(&description, MAX_DESCRIPTION_BYTES),
                    "channel description exceeds {MAX_DESCRIPTION_BYTES} bytes as JSON text"
                );
                anyhow::ensure!(
                    !team.board.0.contains_channel(&channel),
                    "channel already exists"
                );
                anyhow::ensure!(
                    team.board
                        .0
                        .channels
                        .keys()
                        .filter(|name| *name != "general")
                        .count()
                        < MAX_CHANNELS - 1,
                    "channel limit reached"
                );
                team.board.edit().channels.insert(
                    channel.clone(),
                    BoardChannel {
                        description: description.clone(),
                        subscribers: BTreeSet::new(),
                    },
                );
                json!({ "channel": channel, "description": description })
            }
            BoardAction::Subscribe { channel } => {
                team.board.edit().subscribe(actor, channel, true)?
            }
            BoardAction::Unsubscribe { channel } => {
                team.board.edit().subscribe(actor, channel, false)?
            }
            BoardAction::Post {
                channel,
                text,
                claimable,
                reply_to,
                notify,
            } => {
                for target in &notify {
                    ensure_active_member(&team, target)?;
                }
                let post = team
                    .board
                    .edit()
                    .add_post(actor, channel, text, claimable, reply_to)?;
                recipients.extend(notify);
                if let Some(channel) = team.board.0.channels.get(&post.channel) {
                    recipients.extend(channel.subscribers.iter().cloned());
                }
                let notice = format!(
                    "New board post {} in #{}. Use agent_message_board read_post to read it.",
                    post.id, post.channel
                );
                append_notices(&mut team, actor, &mut recipients, &notice);
                json!(post)
            }
            BoardAction::Claim { post_id } => {
                let claimable = match &team.board.0.post(post_id)?.work {
                    Some(BoardWork::Open) => true,
                    Some(BoardWork::Claimed { owner }) => !team.members.iter().any(|member| {
                        member.id == *owner && member.status != TeamMemberStatus::Closed
                    }),
                    _ => false,
                };
                anyhow::ensure!(claimable, "post is not open for claims");
                let post = team.board.edit().post_mut(post_id)?;
                post.work = Some(BoardWork::Claimed {
                    owner: actor.to_owned(),
                });
                json!(post)
            }
            BoardAction::Release { post_id } => {
                let post = team.board.edit().owned_post(post_id, actor)?;
                post.work = Some(BoardWork::Open);
                json!(post)
            }
            BoardAction::Handoff { post_id, target } => {
                anyhow::ensure!(target != actor, "handoff target must be another agent");
                ensure_active_member(&team, &target)?;
                let post = team.board.edit().owned_post(post_id, actor)?;
                post.work = Some(BoardWork::Claimed {
                    owner: target.clone(),
                });
                let notice = format!(
                    "{} was handed off to you in #{}. Use agent_message_board read_post to read the assignment.",
                    post.id, post.channel
                );
                let value = json!(post);
                recipients.insert(target);
                append_notices(&mut team, actor, &mut recipients, &notice);
                value
            }
            BoardAction::Complete { post_id, evidence } => {
                validate_text(&evidence)?;
                let post = team.board.edit().owned_post(post_id, actor)?;
                post.work = Some(BoardWork::Completed {
                    owner: actor.to_owned(),
                    evidence,
                });
                json!(post)
            }
        };
        team.updated_at = OffsetDateTime::now_utc();
        // Board updates and their mailbox records share one atomic snapshot.
        self.persist(&team).await?;
        teams.insert(team.id.clone(), team);
        Ok(BoardResult { value, recipients })
    }
}

impl BoardState {
    fn contains_channel(&self, channel: &str) -> bool {
        channel == "general" || self.channels.contains_key(channel)
    }

    fn channels_for(&self, actor: &str) -> Value {
        let mut names = self
            .channels
            .keys()
            .map(String::as_str)
            .collect::<BTreeSet<_>>();
        names.insert("general");
        let channels = names.into_iter().map(|name| {
            let channel = self.channels.get(name);
            json!({
                "channel": name,
                "description": channel.map_or("", |channel| channel.description.as_str()),
                "subscribed": channel.is_some_and(|channel| channel.subscribers.contains(actor)),
            })
        }).collect::<Vec<_>>();
        json!({ "channels": channels })
    }

    fn subscribe(
        &mut self,
        actor: &str,
        channel: String,
        subscribed: bool,
    ) -> anyhow::Result<Value> {
        anyhow::ensure!(
            self.contains_channel(&channel),
            "unknown channel {channel:?}"
        );
        let entry = self.channels.entry(channel.clone()).or_default();
        if subscribed {
            entry.subscribers.insert(actor.to_owned());
        } else {
            entry.subscribers.remove(actor);
        }
        Ok(json!({ "channel": channel, "subscribed": subscribed }))
    }

    fn post(&self, id: Uuid) -> anyhow::Result<&BoardPost> {
        self.posts
            .iter()
            .find(|post| post.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown board post {id}"))
    }

    fn post_mut(&mut self, id: Uuid) -> anyhow::Result<&mut BoardPost> {
        self.posts
            .iter_mut()
            .find(|post| post.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown board post {id}"))
    }

    fn owned_post(&mut self, id: Uuid, actor: &str) -> anyhow::Result<&mut BoardPost> {
        let post = self.post_mut(id)?;
        anyhow::ensure!(
            matches!(&post.work, Some(BoardWork::Claimed { owner }) if owner == actor),
            "only the current claimant may update this post"
        );
        Ok(post)
    }

    fn add_post(
        &mut self,
        actor: &str,
        channel: String,
        text: String,
        claimable: bool,
        reply_to: Option<Uuid>,
    ) -> anyhow::Result<BoardPost> {
        anyhow::ensure!(
            self.contains_channel(&channel),
            "unknown channel {channel:?}"
        );
        validate_text(&text)?;
        anyhow::ensure!(self.posts.len() < MAX_POSTS, "board post limit reached");
        if let Some(parent) = reply_to {
            anyhow::ensure!(
                self.post(parent)?.channel == channel,
                "replies must use the original channel"
            );
        }
        let post = BoardPost {
            id: Uuid::new_v4(),
            channel,
            author: actor.to_owned(),
            text,
            reply_to,
            work: claimable.then_some(BoardWork::Open),
            created_at: OffsetDateTime::now_utc(),
        };
        self.posts.push(post.clone());
        Ok(post)
    }
}

fn general_channel() -> String {
    "general".to_owned()
}

fn validate_channel(channel: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        (1..=48).contains(&channel.len())
            && channel.as_bytes()[0].is_ascii_lowercase()
            && channel.bytes().all(|byte| byte.is_ascii_lowercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'-' | b'_')),
        "use a channel name of 1 to 48 lowercase ASCII letters, digits, hyphens or underscores, starting with a letter"
    );
    Ok(())
}

fn validate_text(text: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        !text.trim().is_empty() && encoded_text_fits(text, MAX_TEXT_BYTES),
        "board text must be nonempty and fit within {MAX_TEXT_BYTES} bytes as JSON text"
    );
    Ok(())
}

fn encoded_text_fits(text: &str, limit: usize) -> bool {
    text.len() <= limit && Value::String(text.to_owned()).to_string().len() <= limit + 2
}

fn ensure_active_member(team: &TeamState, member_id: &str) -> anyhow::Result<()> {
    anyhow::ensure!(
        team.members
            .iter()
            .any(|member| member.id == member_id && member.status != TeamMemberStatus::Closed),
        "agent must be a non-closed member of this team"
    );
    Ok(())
}

fn append_notices(
    team: &mut TeamState,
    actor: &str,
    recipients: &mut BTreeSet<TeamMemberId>,
    text: &str,
) {
    recipients.retain(|id| id != actor && ensure_active_member(team, id).is_ok());
    for recipient in recipients.iter() {
        team.push_mailbox_message(
            Some(actor.to_owned()),
            recipient.clone(),
            roder_api::teams::TeamMailboxMessageKind::Message,
            text.to_owned(),
        );
    }
}
