//! Common client funcionalities.
extern crate futures;
extern crate librdkafka_sys as rdkafka;

use std::ffi::CString;
use std::os::raw::c_void;
use std::collections::HashMap;
use std::marker::PhantomData;

use self::futures::Complete;

use config::Config;
use error::{IsError, Error};
use util::cstr_to_owned;

/// Specifies the type of client.
pub enum ClientType {
    /// A librdkafka consumer
    Consumer,
    /// A librdkafka producer
    Producer,
}

/// A librdkafka client.
pub struct Client {
    pub ptr: *mut rdkafka::rd_kafka_t,
}

unsafe impl Sync for Client {}
unsafe impl Send for Client {}

#[derive(Debug)]
/// Information returned by the producer after a message has been delivered
/// or failed to be delivered.
pub struct DeliveryStatus {
    error: rdkafka::rd_kafka_resp_err_t,
    partition: i32,
    offset: i64,
}

unsafe extern "C" fn prod_callback(_client: *mut rdkafka::rd_kafka_t,
                                   msg: *const rdkafka::rd_kafka_message_t,
                                   _opaque: *mut c_void) {
    let tx = Box::from_raw((*msg)._private as *mut Complete<DeliveryStatus>);
    let delivery_status = DeliveryStatus {
        error: (*msg).err,
        partition: (*msg).partition,
        offset: (*msg).offset,
    };
    // TODO: add topic name?
    trace!("Delivery event received: {:?}", delivery_status);
    tx.complete(delivery_status);
}

impl Client {
    pub fn new(config: &Config, client_type: ClientType) -> Result<Client, Error> {
        let errstr = [0i8; 1024];
        let config_ptr = try!(config.create_kafka_config());
        let rd_kafka_type = match client_type {
            ClientType::Consumer => rdkafka::rd_kafka_type_t::RD_KAFKA_CONSUMER,
            ClientType::Producer => {
                unsafe { rdkafka::rd_kafka_conf_set_dr_msg_cb(config_ptr, Some(prod_callback)) };
                rdkafka::rd_kafka_type_t::RD_KAFKA_PRODUCER
            }
        };
        let client_ptr =
            unsafe { rdkafka::rd_kafka_new(rd_kafka_type, config_ptr, errstr.as_ptr() as *mut i8, errstr.len()) };
        if client_ptr.is_null() {
            return Err(Error::ClientCreation(cstr_to_owned(&errstr)));
        }
        Ok(Client { ptr: client_ptr })
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        trace!("Destroy rd_kafka");
        unsafe { rdkafka::rd_kafka_destroy(self.ptr) };
        unsafe { rdkafka::rd_kafka_wait_destroyed(1000) };
    }
}

/// Builder for a Topic.
pub struct TopicBuilder<'a> {
    name: String,
    conf: HashMap<String, String>,
    client: &'a Client,
}

/// Represents a Kafka topic, associated to a producer.
pub struct Topic<'a> {
    pub ptr: *mut rdkafka::rd_kafka_topic_t,
    phantom: PhantomData<&'a u8>, // Refers to client
}


impl<'a> TopicBuilder<'a> {
    pub fn new(client: &'a Client, name: &str) -> TopicBuilder<'a> {
        TopicBuilder {
            name: name.to_string(),
            client: client,
            conf: HashMap::new(),
        }
    }

    pub fn set<'b>(&'b mut self, key: &str, value: &str) -> &'b mut TopicBuilder<'a> {
        self.conf.insert(key.to_string(), value.to_string());
        self
    }

    pub fn create(&self) -> Result<Topic<'a>, Error> {
        let name_ptr = CString::new(self.name.clone()).unwrap();
        let config_ptr = unsafe { rdkafka::rd_kafka_topic_conf_new() };
        let errstr = [0; 1024];
        for (name, value) in &self.conf {
            let name_c = try!(CString::new(name.to_string()));
            let value_c = try!(CString::new(value.to_string()));
            let ret = unsafe {
                rdkafka::rd_kafka_topic_conf_set(config_ptr,
                                                 name_c.as_ptr(),
                                                 value_c.as_ptr(),
                                                 errstr.as_ptr() as *mut i8,
                                                 errstr.len())
            };
            if ret.is_error() {
                let descr = cstr_to_owned(&errstr);
                return Err(Error::Config((ret, descr, name.to_string(), value.to_string())));
            }
        }
        let topic_ptr = unsafe { rdkafka::rd_kafka_topic_new(self.client.ptr, name_ptr.as_ptr(), config_ptr) };
        if topic_ptr.is_null() {
            Err(Error::TopicName(self.name.clone())) //TODO: TopicCreationError
        } else {
            let topic = Topic {
                ptr: topic_ptr,
                phantom: PhantomData,
            };
            Ok(topic)
        }
    }
}

impl<'a> Drop for Topic<'a> {
    fn drop(&mut self) {
        trace!("Destroy rd_kafka_topic");
        unsafe {
            rdkafka::rd_kafka_topic_destroy(self.ptr);
        }
    }
}
