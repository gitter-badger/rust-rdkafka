//! Configuration to create a Consumer or Producer.
extern crate librdkafka_sys as rdkafka;

use std::collections::HashMap;
use std::ffi::CString;
use util::cstr_to_owned;

use error::{Error, IsError};

/// Client configuration.
#[derive(Debug)]
pub struct Config {
    conf: HashMap<String, String>,
}

impl Config {
    /// Creates a new empty configuration.
    pub fn new() -> Config {
        Config { conf: HashMap::new() }
    }

    /// Set a new parameter in the configuration.
    pub fn set<'a>(&'a mut self, key: &str, value: &str) -> &'a mut Config {
        self.conf.insert(key.to_string(), value.to_string());
        self
    }

    /// Creates the native rdkafka configuration.
    pub fn create_kafka_config(&self) -> Result<*mut rdkafka::rd_kafka_conf_t, Error> {
        let conf = unsafe { rdkafka::rd_kafka_conf_new() };
        let errstr = [0; 1024];
        for (key, value) in &self.conf {
            let key_c = try!(CString::new(key.to_string()));
            let value_c = try!(CString::new(value.to_string()));
            let ret = unsafe {
                rdkafka::rd_kafka_conf_set(conf,
                                           key_c.as_ptr(),
                                           value_c.as_ptr(),
                                           errstr.as_ptr() as *mut i8,
                                           errstr.len())
            };
            if ret.is_error() {
                let descr = cstr_to_owned(&errstr);
                return Err(Error::Config((ret, descr, key.to_string(), value.to_string())));
            }
        }
        Ok(conf)
    }
}

/// Create a new consumer.
pub trait CreateConsumer<T, E> {
    fn create_consumer(&self) -> Result<T, E>;
}

/// Create a new producer.
pub trait CreateProducer<T, E> {
    fn create_producer(&self) -> Result<T, E>;
}
