//! Producer implementations.
extern crate librdkafka_sys as rdkafka;
extern crate errno;
extern crate futures;

use self::futures::{Canceled, Future, Poll, Oneshot};

use std::os::raw::c_void;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::thread;

use config::CreateProducer;
use config::Config;
use error::Error;
use message::ToBytes;
use client::{Client, ClientType, TopicBuilder, Topic, DeliveryStatus};


/// Contains a reference counted producer client. It can be safely cloned to
/// create another reference to the same producer.
#[derive(Clone)]
pub struct Producer {
    client: Arc<Client>,
}

impl CreateProducer<Producer, Error> for Config {
    fn create_producer(&self) -> Result<Producer, Error> {
        let client = try!(Client::new(&self, ClientType::Producer));
        let producer = Producer { client: Arc::new(client) };
        Ok(producer)
    }
}

/// A future that will receive a `DeliveryStatus` containing information on the
/// delivery status of the message.
pub struct DeliveryFuture {
    rx: Oneshot<DeliveryStatus>,
}

impl Future for DeliveryFuture {
    type Item = DeliveryStatus;
    type Error = Canceled;

    fn poll(&mut self) -> Poll<DeliveryStatus, Canceled> {
        self.rx.poll()
    }
}

impl Producer {
    /// Return a topic builder associated to the producer.
    pub fn get_topic(&self, topic_name: &str) -> TopicBuilder {
        TopicBuilder::new(&self.client, topic_name)
    }

    /// Poll the producer. Regular calls to `poll` are required to process the evens
    /// and execute the message delivery callbacks.
    pub fn poll(&self, timeout_ms: i32) -> i32 {
        unsafe { rdkafka::rd_kafka_poll(self.client.ptr, timeout_ms) }
    }

    fn _send_copy(&self, topic: &Topic, payload: Option<&[u8]>, key: Option<&[u8]>) -> Result<DeliveryFuture, Error> {
        let (payload_n, plen) = match payload {
            None => (ptr::null_mut(), 0),
            Some(p) => (p.as_ptr() as *mut c_void, p.len()),
        };
        let (key_n, klen) = match key {
            None => (ptr::null_mut(), 0),
            Some(k) => (k.as_ptr() as *mut c_void, k.len()),
        };
        let (tx, rx) = futures::oneshot();
        let boxed_tx = Box::new(tx);
        let n = unsafe {
            rdkafka::rd_kafka_produce(topic.ptr, -1, rdkafka::RD_KAFKA_MSG_F_COPY as i32, payload_n, plen,
                                      key_n, klen, Box::into_raw(boxed_tx) as *mut c_void)
        };
        if n != 0 {
            let errno = errno::errno().0 as i32;
            let kafka_error = unsafe { rdkafka::rd_kafka_errno2err(errno) };
            Err(Error::MessageProduction(kafka_error))
        } else {
            Ok(DeliveryFuture { rx: rx })
        }
    }

    /// Send a copy of the message and key provided. Return a `DeliveryFuture` or an `Error`.
    pub fn send_copy<P, K>(&self, topic: &Topic, payload: Option<&P>, key: Option<&K>) -> Result<DeliveryFuture, Error>
        where K: ToBytes,
              P: ToBytes {
        self._send_copy(topic, payload.map(P::to_bytes), key.map(K::to_bytes))
    }

    /// Start the polling thread for the producer. It returns a `ProducerPollingThread` that will
    /// process al the events. Calling `poll` is not required if the `ProducerPollingThread`
    /// thread is running.
    pub fn start_polling_thread(&self) -> ProducerPollingThread {
        let mut threaded_producer = ProducerPollingThread::new(self);
        threaded_producer.start();
        threaded_producer
    }
}


/// A producer with an internal running thread. This producer doesn't neeed to be polled.
/// The internal thread can be terminated with the `stop` method or moving the
/// `ProducerPollingThread` out of scope.
#[must_use = "Producer polling thread will stop immediately if unused"]
pub struct ProducerPollingThread {
    producer: Producer,
    should_stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl ProducerPollingThread {
    /// Creates a new producer. The internal thread will not be running yet.
    pub fn new(producer: &Producer) -> ProducerPollingThread {
        ProducerPollingThread {
            producer: producer.clone(),
            should_stop: Arc::new(AtomicBool::new(false)),
            handle: None,
        }
    }

    /// Start the internal polling thread.
    pub fn start(&mut self) {
        let producer = self.producer.clone();
        let should_stop = self.should_stop.clone();
        let handle = thread::Builder::new()
            .name("polling thread".to_string())
            .spawn(move || {
                trace!("Polling thread loop started");
                while !should_stop.load(Ordering::Relaxed) {
                    let n = producer.poll(100);
                    if n != 0 {
                        trace!("Receved {} events", n);
                    }
                }
                trace!("Polling thread loop terminated");
            })
            .expect("Failed to start polling thread");
        self.handle = Some(handle);
    }

    /// Stop the internal polling thread. The thread can also be stopped by moving
    /// the ProducerPollingThread out of scope.
    pub fn stop(&mut self) {
        if self.handle.is_some() {
            trace!("Stopping polling");
            self.should_stop.store(true, Ordering::Relaxed);
            trace!("Waiting for polling thread termination");
            match self.handle.take().unwrap().join() {
                Ok(()) => trace!("Polling stopped"),
                Err(e) => warn!("Failure while terminating thread: {:?}", e),
            };
        }
    }
}

impl Drop for ProducerPollingThread {
    fn drop(&mut self) {
        trace!("Destroy ProducerPollingThread");
        self.stop();
    }
}
