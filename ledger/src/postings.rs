//! Relays the database's notice of a posted entry to the subscribers of the
//! entry's company (Bilimbi `Ledger.Postings`).
//!
//! The `finance_ledger_entries_notify` trigger calls `pg_notify` in the
//! posting transaction, and PostgreSQL delivers a notification only when that
//! transaction commits: a rolled-back post announces nothing, and a post
//! inside a caller's own transaction is announced at that transaction's
//! commit, after the entry is readable. One dedicated connection per process
//! LISTENs, opens with the first subscription (never at boot), and a database
//! that cannot be reached is retried rather than allowed to end the relay.
use crate::PostingEvent;
use std::{
    future::poll_fn,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::sync::{broadcast, watch};
use tokio_postgres::{AsyncMessage, NoTls};

const CHANNEL: &str = "finance_ledger_postings";
const RETRY: Duration = Duration::from_secs(5);
/// How long a subscription waits for the relay's connection to listen.
const LISTEN_WAIT: Duration = Duration::from_secs(10);
const BACKLOG: usize = 1024;

/// What the relay hears: a posted entry of a tenant's company, or word that
/// notices may have been missed.
#[derive(Clone, Debug)]
pub(crate) enum Heard {
    Posted {
        tenant: i64,
        company_id: i64,
        entry_number: i64,
    },
    Missed,
}

/// The process's relay, shared by every subscriber.
#[derive(Clone)]
pub(crate) struct Relay {
    heard: broadcast::Sender<Heard>,
    listening: watch::Sender<bool>,
    started: Arc<AtomicBool>,
}

impl Relay {
    pub(crate) fn new() -> Self {
        Self {
            heard: broadcast::channel(BACKLOG).0,
            listening: watch::channel(false).0,
            started: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Starts listening if this is the first subscription, and waits until
    /// the connection listens, so a subscriber hears every posting committed
    /// after this returns. `None` when the database cannot be reached in time.
    // TODO(platform): base/database exposes only the pool, not its connection
    // settings; the relay opens its own connection from DATABASE_URL, the
    // variable the Platform's pool is built from.
    pub(crate) async fn subscribe(
        &self,
        tenant: i64,
        company_id: i64,
    ) -> Option<crate::PostingSubscription> {
        let receiver = self.heard.subscribe();
        if !self.started.swap(true, Ordering::SeqCst) {
            tokio::spawn(self.clone().run());
        }
        let mut listening = self.listening.subscribe();
        tokio::time::timeout(LISTEN_WAIT, listening.wait_for(|up| *up))
            .await
            .ok()?
            .ok()?;
        Some(crate::PostingSubscription {
            tenant,
            company_id,
            receiver,
        })
    }

    async fn run(self) {
        loop {
            let _ = self.listen_once().await;
            let _ = self.listening.send(false);
            // Notices committed while disconnected are gone: subscribers re-read.
            let _ = self.heard.send(Heard::Missed);
            tokio::time::sleep(RETRY).await;
        }
    }

    /// One connection's life: connect, LISTEN, forward until it closes.
    async fn listen_once(&self) -> Option<()> {
        let url = std::env::var("DATABASE_URL").ok()?;
        let (client, mut connection) = tokio_postgres::connect(&url, NoTls).await.ok()?;
        let heard = self.heard.clone();
        let driver = tokio::spawn(async move {
            while let Some(Ok(message)) = poll_fn(|cx| connection.poll_message(cx)).await {
                if let AsyncMessage::Notification(notice) = message {
                    if let Some(posted) = parse(notice.payload()) {
                        let _ = heard.send(posted);
                    }
                }
            }
        });
        client
            .batch_execute(&format!("LISTEN {CHANNEL}"))
            .await
            .ok()?;
        let _ = self.listening.send(true);
        let _ = driver.await;
        Some(())
    }
}

/// `tenant_id:company_id:entry_number`, or nothing for a payload that is not.
fn parse(payload: &str) -> Option<Heard> {
    let mut parts = payload.split(':').map(|part| part.parse::<i64>().ok());
    let (tenant, company_id, entry_number) = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(Heard::Posted {
        tenant,
        company_id,
        entry_number,
    })
}

impl crate::PostingSubscription {
    /// The next notice for the subscribed company, waiting for one. `None`
    /// once the relay has gone. A notice carries no figures: read them
    /// through the facade, which authorizes the read afresh.
    pub async fn next(&mut self) -> Option<PostingEvent> {
        loop {
            match self.receiver.recv().await {
                Ok(Heard::Posted {
                    tenant,
                    company_id,
                    entry_number,
                }) if tenant == self.tenant && company_id == self.company_id => {
                    return Some(PostingEvent::Posted {
                        company_id,
                        entry_number,
                    });
                }
                Ok(Heard::Posted { .. }) => {}
                Ok(Heard::Missed) | Err(broadcast::error::RecvError::Lagged(_)) => {
                    return Some(PostingEvent::Refresh);
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }
}
