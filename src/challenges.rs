//! Private, opt-in challenges and cooperative goals. Reviews count only after joining.
use crate::competition::Participant;
use crate::store::Store;
use rusqlite::{Connection, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const DAY_MS: i64 = 86_400_000;

#[derive(Debug)]
pub enum Error {
    Invalid(&'static str),
    Forbidden,
    NotFound,
    Storage(rusqlite::Error),
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Storage(value)
    }
}

#[derive(Clone, Deserialize, Serialize, Debug, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    StudyDays,
    Reviews,
}

impl Kind {
    fn key(&self) -> &'static str {
        match self {
            Self::StudyDays => "study_days",
            Self::Reviews => "reviews",
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
pub struct Create {
    pub title: String,
    pub kind: Kind,
    pub cooperative: bool,
    pub target: u64,
    pub duration_days: u32,
    pub recipients: Vec<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Action {
    pub action: String,
}

#[derive(Serialize, Debug)]
pub struct Member {
    pub user: String,
    pub display: String,
    pub status: String,
    pub progress: u64,
}

#[derive(Serialize, Debug)]
pub struct Challenge {
    pub id: i64,
    pub title: String,
    pub kind: Kind,
    pub cooperative: bool,
    pub creator: String,
    pub start_at: i64,
    pub end_at: i64,
    pub target: u64,
    pub members: Vec<Member>,
    pub progress: u64,
    pub status: String,
}

pub fn initialize(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "create table if not exists community_challenges (
           id integer primary key, creator text not null, title text not null,
           kind text not null, cooperative integer not null, target integer not null,
           start_at integer not null, end_at integer not null, cancelled integer not null default 0
         );
         create table if not exists community_members (
           challenge integer not null, user text not null, status text not null,
           joined_at integer, left_at integer,
           primary key (challenge, user),
           foreign key (challenge) references community_challenges(id)
         );
         create index if not exists community_members_user on community_members(user);",
    )
}

pub fn create(
    store: &mut Store,
    user: &str,
    request: &Create,
    eligible: &BTreeSet<String>,
    now: i64,
) -> Result<i64, Error> {
    let title = request.title.trim();
    if title.is_empty() || title.chars().count() > 80 || title.chars().any(char::is_control) {
        return Err(Error::Invalid("Use a title between 1 and 80 characters."));
    }
    if !(1..=31).contains(&request.duration_days) || !(1..=100_000).contains(&request.target) {
        return Err(Error::Invalid(
            "Choose 1–31 days and a target between 1 and 100,000.",
        ));
    }
    let recipients: BTreeSet<_> = request.recipients.iter().cloned().collect();
    if recipients.is_empty()
        || recipients.len() > 30
        || recipients.len() != request.recipients.len()
        || recipients.contains(user)
        || recipients.iter().any(|name| !eligible.contains(name))
    {
        return Err(Error::Invalid(
            "Select 1–30 different people from the available players.",
        ));
    }
    // A rolling duration may straddle an extra local day; targets still stay attainable
    // without relying on that partial day, or on reviews before acceptance.
    let max_days = u64::from(request.duration_days)
        * if request.cooperative {
            recipients.len() as u64 + 1
        } else {
            1
        };
    if request.kind == Kind::StudyDays && request.target > max_days {
        return Err(Error::Invalid(
            "The study-day target exceeds the available participant days.",
        ));
    }
    // Completed goals remain open until the creator closes them or their
    // original deadline passes; the UI permits closing a completed goal.
    let active: i64 = store.conn.query_row(
        "select count(*) from community_challenges where creator=?1 and end_at>?2 and cancelled=0",
        params![user, now],
        |row| row.get(0),
    )?;
    if active >= 10 {
        return Err(Error::Invalid(
            "You already have 10 open challenges. Close or cancel one first.",
        ));
    }
    let tx = store.conn.transaction()?;
    tx.execute(
        "insert into community_challenges (creator,title,kind,cooperative,target,start_at,end_at)
         values (?1,?2,?3,?4,?5,?6,?7)",
        params![
            user,
            title,
            request.kind.key(),
            request.cooperative,
            request.target as i64,
            now,
            now + i64::from(request.duration_days) * DAY_MS
        ],
    )?;
    let id = tx.last_insert_rowid();
    tx.execute(
        "insert into community_members (challenge,user,status,joined_at) values (?1,?2,'accepted',?3)",
        params![id, user, now],
    )?;
    for recipient in recipients {
        tx.execute(
            "insert into community_members (challenge,user,status) values (?1,?2,'invited')",
            params![id, recipient],
        )?;
    }
    tx.commit()?;
    Ok(id)
}

pub fn act(store: &mut Store, user: &str, id: i64, action: &str, now: i64) -> Result<(), Error> {
    let row: Option<(String, i64, bool)> = store
        .conn
        .query_row(
            "select creator,end_at,cancelled from community_challenges where id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    let (creator, end, cancelled) = row.ok_or(Error::NotFound)?;
    let membership: Option<String> = store
        .conn
        .query_row(
            "select status from community_members where challenge=?1 and user=?2",
            params![id, user],
            |r| r.get(0),
        )
        .optional()?;
    let status = membership.ok_or(Error::Forbidden)?;
    if cancelled || now >= end {
        return Err(Error::Invalid("This challenge has ended."));
    }
    match action {
        "cancel" if creator == user => {
            store.conn.execute(
                "update community_challenges set cancelled=1,end_at=min(end_at,?2) where id=?1",
                params![id, now],
            )?;
        }
        "accept" if status == "invited" => {
            store.conn.execute(
                "update community_members set status='accepted',joined_at=?3 where challenge=?1 and user=?2 and status='invited'",
                params![id,user,now],
            )?;
        }
        "decline" if status == "invited" => {
            store.conn.execute(
                "update community_members set status='declined',left_at=?3 where challenge=?1 and user=?2",
                params![id,user,now],
            )?;
        }
        "leave" if status == "accepted" && creator != user => {
            store.conn.execute(
                "update community_members set status='left',left_at=?3 where challenge=?1 and user=?2",
                params![id,user,now],
            )?;
        }
        "cancel" => return Err(Error::Forbidden),
        _ => {
            return Err(Error::Invalid(
                "That action is not available for this invitation.",
            ));
        }
    }
    Ok(())
}

pub fn list(
    store: &Store,
    user: &str,
    players: &[Participant],
    now: i64,
) -> Result<Vec<Challenge>, Error> {
    // Keep every open invitation actionable; only completed history is bounded.
    // Reached goals are still open until cancelled or their deadline passes.
    let mut query = store.conn.prepare(
        "select c.id,c.title,c.kind,c.cooperative,c.creator,c.start_at,c.end_at,c.target,c.cancelled
         from community_challenges c join community_members m on m.challenge=c.id
         where m.user=?1 and m.status in ('accepted','invited')
           and ((c.cancelled=0 and c.end_at>?2) or c.id in (
             select history.id from community_challenges history
             join community_members membership on membership.challenge=history.id
             where membership.user=?1 and membership.status in ('accepted','invited')
               and (history.cancelled<>0 or history.end_at<=?2)
             order by history.id desc limit 100
           ))
         order by c.id desc",
    )?;
    let rows = query
        .query_map(params![user, now], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, bool>(3)?,
                r.get::<_, String>(4)?,
                r.get::<_, i64>(5)?,
                r.get::<_, i64>(6)?,
                u64::from(r.get::<_, u32>(7)?),
                r.get::<_, bool>(8)?,
            ))
        })?
        .collect::<Result<Vec<_>, _>>()?;
    let mut challenges = Vec::new();
    for (id, title, kind, cooperative, creator, start_at, end_at, target, cancelled) in rows {
        let kind = if kind == "study_days" {
            Kind::StudyDays
        } else {
            Kind::Reviews
        };
        let mut query = store.conn.prepare(
            "select user,status,joined_at from community_members where challenge=?1 order by user",
        )?;
        let members = query
            .query_map([id], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, Option<i64>>(2)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let members: Vec<Member> = members
            .into_iter()
            .map(|(member, status, joined)| {
                let player = players.iter().find(|p| p.user == member);
                let mut count = 0;
                let mut days = BTreeSet::new();
                if status == "accepted"
                    && let (Some(player), Some(joined)) = (player, joined)
                {
                    for review in &player.reviews {
                        if review.id >= joined.max(start_at)
                            && review.id < end_at
                            && review.id <= now
                        {
                            count += 1;
                            days.insert(player.clock.day(review.id));
                        }
                    }
                }
                Member {
                    display: player.map_or_else(|| member.clone(), |p| p.display.clone()),
                    user: member,
                    status,
                    progress: if kind == Kind::StudyDays {
                        days.len() as u64
                    } else {
                        count
                    },
                }
            })
            .collect();
        let progress = members.iter().map(|m| m.progress).sum();
        let complete = if cooperative {
            progress >= target
        } else {
            !members.iter().any(|m| m.status == "invited")
                && members
                    .iter()
                    .filter(|m| m.status == "accepted")
                    .all(|m| m.progress >= target)
        };
        let status = if cancelled {
            "cancelled"
        } else if complete {
            "complete"
        } else if now >= end_at {
            "ended"
        } else if now < start_at {
            "upcoming"
        } else {
            "active"
        };
        challenges.push(Challenge {
            id,
            title,
            kind,
            cooperative,
            creator,
            start_at,
            end_at,
            target,
            members,
            progress,
            status: status.into(),
        });
    }
    Ok(challenges)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::game::{Clock, FreezePolicy, Review};

    fn player(user: &str, times: &[i64]) -> Participant {
        Participant {
            user: user.into(),
            display: user.into(),
            clock: Clock::default(),
            freeze_policy: FreezePolicy::default(),
            reviews: times
                .iter()
                .enumerate()
                .map(|(i, id)| Review {
                    id: *id,
                    cid: i as i64,
                    last_ivl: 1,
                    time_ms: 5000,
                    kind: 1,
                })
                .collect(),
        }
    }

    fn request() -> Create {
        Create {
            title: "Study together".into(),
            kind: Kind::Reviews,
            cooperative: true,
            target: 3,
            duration_days: 7,
            recipients: vec!["friend".into()],
        }
    }

    #[test]
    fn invitations_are_private_and_only_post_acceptance_reviews_count() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into(), "other".into()]);
        let id = create(&mut store, "owner", &request(), &eligible, 100).unwrap();
        let players = [
            player("owner", &[90, 101]),
            player("friend", &[110, 201, 202]),
        ];
        assert!(list(&store, "other", &players, 300).unwrap().is_empty());
        assert!(matches!(
            act(&mut store, "other", id, "accept", 150),
            Err(Error::Forbidden)
        ));
        assert_eq!(
            list(&store, "friend", &players, 150).unwrap()[0].progress,
            1
        );
        act(&mut store, "friend", id, "accept", 200).unwrap();
        let result = list(&store, "owner", &players, 300).unwrap();
        assert_eq!(result[0].progress, 3);
        assert_eq!(result[0].status, "complete");
        assert!(matches!(
            act(&mut store, "friend", id, "accept", 250),
            Err(Error::Invalid(_))
        ));
        assert!(matches!(
            act(&mut store, "friend", id, "cancel", 250),
            Err(Error::Forbidden)
        ));
        act(&mut store, "friend", id, "leave", 250).unwrap();
        assert!(list(&store, "friend", &players, 300).unwrap().is_empty());
        assert_eq!(list(&store, "owner", &players, 300).unwrap()[0].progress, 1);
    }

    #[test]
    fn study_days_are_distinct_and_future_and_post_deadline_reviews_do_not_count() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let mut request = request();
        request.kind = Kind::StudyDays;
        request.target = 3;
        let eligible = BTreeSet::from(["friend".into()]);
        let start = DAY_MS + 12 * 3_600_000;
        let id = create(&mut store, "owner", &request, &eligible, start).unwrap();
        let players = [player(
            "owner",
            &[start, start + 100, start + DAY_MS, start + 8 * DAY_MS],
        )];
        assert_eq!(
            list(&store, "owner", &players, start + 1000).unwrap()[0].progress,
            1
        );
        assert_eq!(
            list(&store, "owner", &players, start + 9 * DAY_MS).unwrap()[0].progress,
            2
        );
        assert!(matches!(
            act(&mut store, "friend", id, "accept", start + 7 * DAY_MS),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn cancelling_preserves_prior_progress_and_excludes_later_reviews() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into()]);
        let id = create(&mut store, "owner", &request(), &eligible, 100).unwrap();
        act(&mut store, "owner", id, "cancel", 200).unwrap();
        let players = [player("owner", &[101, 199, 200, 201, 500])];
        let result = list(&store, "owner", &players, 1000).unwrap();
        assert_eq!(result[0].status, "cancelled");
        assert_eq!(result[0].end_at, 200);
        assert_eq!(
            result[0].progress, 2,
            "the cancellation instant is an exclusive cutoff"
        );
        assert!(matches!(
            act(&mut store, "friend", id, "accept", 250),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn creator_can_close_a_completed_goal_to_free_an_open_slot() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into()]);
        let mut ids = Vec::new();
        for _ in 0..10 {
            ids.push(create(&mut store, "owner", &request(), &eligible, 100).unwrap());
        }
        let players = [player("owner", &[101, 102, 103])];
        assert!(
            list(&store, "owner", &players, 150)
                .unwrap()
                .iter()
                .all(|goal| goal.status == "complete")
        );
        assert!(matches!(
            create(&mut store, "owner", &request(), &eligible, 150),
            Err(Error::Invalid(_))
        ));
        act(&mut store, "owner", ids[0], "cancel", 200).unwrap();
        assert!(create(&mut store, "owner", &request(), &eligible, 201).is_ok());
    }

    #[test]
    fn validates_targets_people_and_creator_limit_before_writing() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into()]);
        let mut request = request();
        request.recipients = vec!["stranger".into()];
        assert!(matches!(
            create(&mut store, "owner", &request, &eligible, 100),
            Err(Error::Invalid(_))
        ));
        request.recipients = vec!["friend".into()];
        request.duration_days = 0;
        assert!(matches!(
            create(&mut store, "owner", &request, &eligible, 100),
            Err(Error::Invalid(_))
        ));
        request.duration_days = 7;
        for _ in 0..10 {
            create(&mut store, "owner", &request, &eligible, 100).unwrap();
        }
        assert!(matches!(
            create(&mut store, "owner", &request, &eligible, 100),
            Err(Error::Invalid(_))
        ));
    }

    #[test]
    fn recent_history_does_not_hide_an_older_open_invitation() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into()]);
        let open = create(&mut store, "owner", &request(), &eligible, 100).unwrap();
        for now in 101..206 {
            let closed = create(&mut store, "owner", &request(), &eligible, now).unwrap();
            act(&mut store, "owner", closed, "cancel", now + 1).unwrap();
        }
        let invitations = list(&store, "friend", &[], 300).unwrap();
        assert_eq!(
            invitations.len(),
            101,
            "one open invitation plus 100 historical goals"
        );
        assert!(invitations.iter().any(|goal| goal.id == open));
        act(&mut store, "friend", open, "accept", 300).unwrap();
        assert!(
            list(&store, "friend", &[], 301)
                .unwrap()
                .iter()
                .any(|goal| {
                    goal.id == open
                        && goal
                            .members
                            .iter()
                            .any(|member| member.user == "friend" && member.status == "accepted")
                })
        );
        act(&mut store, "friend", open, "leave", 302).unwrap();
        assert_eq!(list(&store, "friend", &[], 303).unwrap().len(), 100);
        assert!(
            list(&store, "owner", &[], 303)
                .unwrap()
                .iter()
                .any(|goal| goal.id == open)
        );
        act(&mut store, "owner", open, "cancel", 304).unwrap();
        assert_eq!(list(&store, "owner", &[], 305).unwrap().len(), 100);
    }

    #[test]
    fn every_open_invitation_is_visible_even_above_the_history_limit() {
        let (mut store, _) = crate::decks::tests::temporary_store();
        initialize(&store.conn).unwrap();
        let eligible = BTreeSet::from(["friend".into()]);
        for index in 0..105 {
            create(
                &mut store,
                &format!("owner-{index}"),
                &request(),
                &eligible,
                100,
            )
            .unwrap();
        }
        let invitations = list(&store, "friend", &[], 200).unwrap();
        assert_eq!(invitations.len(), 105);
        assert!(invitations.iter().all(|goal| goal.status == "active"));
    }
}
