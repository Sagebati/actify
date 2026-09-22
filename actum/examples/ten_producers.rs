//! Ten producers, one actor.
//!
//! Ten tasks each hold a clone of one handle and record into a single
//! `Ledger`. The actor is the one consumer: it runs one call at a time, so the
//! counts it keeps need no lock, and ten producers outrunning its queue wait
//! for it rather than piling up without limit.
//!
//! Run it with `cargo run --example ten_producers`.

use actum::actum;

const PRODUCERS: usize = 10;
const ITEMS_EACH: usize = 100;
const QUEUE: usize = 8;

/// The one consumer's state: how many items each producer has delivered.
struct Ledger {
    counts: [usize; PRODUCERS],
}

#[actum]
impl Ledger {
    /// Records one item from `producer`. `&mut self` and no lock, because the
    /// actor runs one call at a time.
    fn record(&mut self, producer: usize) {
        self.counts[producer] += 1;
    }

    fn counts(&self) -> [usize; PRODUCERS] {
        self.counts
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() {
    // A bounded channel. Ten producers outrun a queue of eight, and when they
    // do, `record` waits for a slot instead of queueing without limit. The
    // ceiling is the channel's own rule - `buffer + one slot per sender` - and
    // every handle is a sender, so eight plus the ten clones made below.
    let (mut handle, actor) = LedgerHandle::builder(Ledger {
        counts: [0; PRODUCERS],
    })
    .channel(futures_channel::mpsc::channel(QUEUE))
    .build();
    tokio::spawn(actor);

    // One handle per producer. A handle sends through `&mut self`, so it
    // carries one call at a time; concurrency is spelled by cloning.
    let producers: Vec<_> = (0..PRODUCERS)
        .map(|id| {
            let mut handle = handle.clone();
            tokio::spawn(async move {
                for _ in 0..ITEMS_EACH {
                    handle.record(id).await;
                }
            })
        })
        .collect();
    for producer in producers {
        producer.await.unwrap();
    }

    // Every producer's calls have returned, so every item is in the ledger.
    let counts = handle.counts().await;
    for (producer, count) in counts.iter().enumerate() {
        println!("producer {producer:>2}: {count} items");
    }
    let total: usize = counts.iter().sum();
    println!("total: {total} of {}", PRODUCERS * ITEMS_EACH);
    assert_eq!(total, PRODUCERS * ITEMS_EACH);
}
