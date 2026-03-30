use crossbeam_queue::ArrayQueue;
use std::sync::Arc;

#[derive(Clone)]
pub struct SampleProducer {
    queue: Arc<ArrayQueue<f32>>,
}

#[derive(Clone)]
pub struct SampleConsumer {
    queue: Arc<ArrayQueue<f32>>,
}

pub fn create_sample_queue(capacity: usize) -> (SampleProducer, SampleConsumer) {
    let capacity = capacity.max(1024);
    let queue = Arc::new(ArrayQueue::new(capacity));
    (
        SampleProducer { queue: Arc::clone(&queue) },
        SampleConsumer { queue },
    )
}

impl SampleProducer {
    /// Push interleaved stereo samples [L, R, L, R, ...] into the queue.
    /// Samples are always pushed as complete L+R pairs - if there is not enough
    /// room for both floats the entire pair is dropped, keeping the queue length
    /// even so the consumer never de-syncs L from R.
    pub fn push_samples(&self, samples: &[f32]) -> u64 {
        let mut dropped = 0_u64;
        for pair in samples.chunks_exact(2) {
            // We are the sole producer, so len() can only decrease (never increase)
            // between this check and the two pushes below - there will always be room.
            if self.queue.len() + 2 > self.queue.capacity() {
                dropped += 2;
                continue;
            }
            let _ = self.queue.push(pair[0]);
            let _ = self.queue.push(pair[1]);
        }
        dropped
    }

    /// Number of complete stereo frames (L+R pairs) currently in the queue.
    pub fn len(&self) -> usize {
        self.queue.len() / 2
    }

    pub fn clear(&self) -> usize {
        let mut removed = 0;
        while self.queue.pop().is_some() {
            removed += 1;
        }
        removed / 2
    }
}

impl SampleConsumer {
    pub fn pop_sample(&self) -> Option<f32> {
        self.queue.pop()
    }
}
