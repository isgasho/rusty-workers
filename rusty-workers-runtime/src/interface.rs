use crate::buffer::*;
use rusty_workers::types::*;
use serde::{Deserialize, Serialize};
use std::cell::Cell;
use std::io::Read;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceCall {
    Sync(SyncCall),
    Async(AsyncCallV),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum SyncCall {
    Log(String),
    Done,
    SendFetchResponse(ResponseObject),
    GetRandomValues,
    GetFile(String),
    Crypto(crate::crypto::CryptoCall),
}

pub struct AsyncCall {
    pub v: AsyncCallV,
    pub buffers: Vec<JsArrayBufferViewRef>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum AsyncCallV {
    SetTimeout(u64),
    Fetch(RequestObject),
    KvGet {
        namespace: String,
        lock: bool,
    },
    KvPut {
        namespace: String,
    },
    KvDelete {
        namespace: String,
    },
    KvScan {
        namespace: String,
        limit: u32,
        lock: bool,
    },
    KvBeginTransaction,
    KvRollbackTransaction,
    KvCommitTransaction,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum ServiceEvent {
    Fetch(FetchEvent),
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct FetchEvent {
    pub request: RequestObject,
}

pub struct ReadableByteCellSlice<'a>(&'a [Cell<u8>]);

impl<'a> ReadableByteCellSlice<'a> {
    pub fn new(inner: &'a [Cell<u8>]) -> Self {
        Self(inner)
    }
}

impl<'a> Read for ReadableByteCellSlice<'a> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let read_len = buf.len().min(self.0.len());
        for i in 0..read_len {
            buf[i] = self.0[i].get();
        }
        self.0 = &self.0[read_len..];
        Ok(read_len)
    }
}
