use std::{
    collections::{HashMap, HashSet, VecDeque},
    fmt::Debug,
    marker::PhantomData,
};

use jsonrpsee::client::Subscription;

use codec::{Codec, Compact, Decode, Encode, Input, Output};
use frame_support::dispatch::DispatchInfo;
use sp_core::storage::StorageChangeSet;
use sp_runtime::{DispatchError, DispatchResult};

use crate::{
    error::{Error, RuntimeError},
    metadata::{EventArg, Metadata},
    runtime::{
        frame::system::{Phase, System},
        Event, Runtime,
    },
};

/// Raw byte for an Event or an Error
pub enum Raw {
    /// Raw bytes for an Event
    Event(RawEvent),
    /// Raw bytes for an Error
    Error(RuntimeError),
}

/// Raw bytes for an Event
pub struct RawEvent {
    /// The name of the module from whence the Event originated
    pub module: String,
    /// The name of the Event
    pub variant: String,
    /// The raw Event data
    pub data: Vec<u8>,
}

impl Debug for RawEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.debug_struct("RawEvent")
            .field("module", &self.module)
            .field("variant", &self.variant)
            .field("data", &hex::encode(&self.data))
            .finish()
    }
}

#[derive(Debug)]
pub struct EventsDecoder<T> {
    _runtime: PhantomData<fn() -> T>, // variant
    metadata: Metadata,
    type_sizes: HashMap<String, usize>,
}

impl<T: System> EventsDecoder<T> {
    /// Creates a new `EventsDecoder`.
    pub fn new(metadata: Metadata) -> Self {
        let mut decoder = Self {
            _runtime: PhantomData,
            metadata,
            type_sizes: HashMap::new(),
        };

        // register default event arg type sizes for dynamic decoding of events
        decoder.register_type_size::<()>("PhantomData");
        decoder.register_type_size::<DispatchInfo>("DispatchInfo");
        decoder.register_type_size::<bool>("bool");
        decoder.register_type_size::<u8>("u8");
        decoder.register_type_size::<u32>("u32");
        decoder.register_type_size::<u64>("u64");
        decoder.register_type_size::<u128>("u128");
        decoder.register_type_size::<u32>("AccountIndex");
        decoder.register_type_size::<u32>("SessionIndex");
        decoder.register_type_size::<u32>("PropIndex");
        decoder.register_type_size::<u32>("ProposalIndex");
        decoder.register_type_size::<u32>("AuthorityIndex");
        decoder.register_type_size::<u64>("AuthorityWeight");
        decoder.register_type_size::<T::Index>("Index");
        decoder.register_type_size::<T::BlockNumber>("BlockNumber");
        decoder.register_type_size::<T::Hash>("Hash");
        decoder.register_type_size::<T::AccountId>("AccountId");
        decoder
    }

    /// Register a type.
    pub fn register_type_size<U>(&mut self, name: &str) -> usize
    where
        U: Default + Codec,
    {
        let size = U::default().encode().len();
        self.type_sizes.insert(name.to_string(), size);
        size
    }

    /// Decode events.
    pub fn decode_events(&self, input: &mut &[u8]) -> Result<Vec<(Phase, Raw)>, Error> {
        let compact_len = <Compact<u32>>::decode(input)?;
        let len = compact_len.0 as usize;

        let mut r = Vec::new();
        for _ in 0..len {
            // decode EventRecord
            let phase = Phase::decode(input)?;
            let module_variant = input.read_byte()?;

            let module = self.metadata.module_with_events(module_variant)?;
            let event_variant = input.read_byte()?;
            let event_metadata = module.event(event_variant)?;
            log::debug!(
                "received event '{}::{}'",
                module.name(),
                event_metadata.name
            );

            let mut event_data = Vec::<u8>::new();
            let result = self.decode_raw_bytes(&event_metadata.arguments(), input, &mut event_data);
            let raw = match result {
                Ok(()) => {
                    log::debug!("raw bytes: {}", hex::encode(&event_data),);

                    let event = RawEvent {
                        module: module.name().to_string(),
                        variant: event_metadata.name.clone(),
                        data: event_data,
                    };

                    // topics come after the event data in EventRecord
                    let _topics = Vec::<T::Hash>::decode(input)?;
                    Raw::Event(event)
                }
                Err(Error::Runtime(err)) => Raw::Error(err),
                Err(err) => return Err(err),
            };

            r.push((phase, raw));
        }
        Ok(r)
    }

    fn decode_raw_bytes<I: Input, W: Output>(
        &self,
        args: &[EventArg],
        input: &mut I,
        output: &mut W,
    ) -> Result<(), Error> {
        for arg in args {
            match arg {
                EventArg::Vec(arg) => {
                    let len = <Compact<u32>>::decode(input)?;
                    len.encode_to(output);
                    for _ in 0..len.0 {
                        self.decode_raw_bytes(&[*arg.clone()], input, output)?
                    }
                }
                EventArg::Option(arg) => match input.read_byte()? {
                    0 => output.push_byte(0),
                    1 => {
                        output.push_byte(1);
                        self.decode_raw_bytes(&[*arg.clone()], input, output)?
                    }
                    _ => return Err(Error::Other("unexpected first byte decoding Option".into())),
                },
                EventArg::Tuple(args) => self.decode_raw_bytes(args, input, output)?,
                EventArg::Primitive(name) => {
                    let result = match name.as_str() {
                        "DispatchResult" => DispatchResult::decode(input)?,
                        "DispatchError" => Err(DispatchError::decode(input)?),
                        _ => {
                            if let Some(size) = self.type_sizes.get(name) {
                                let mut buf = vec![0; *size];
                                input.read(&mut buf)?;
                                output.write(&buf);
                                Ok(())
                            } else {
                                return Err(Error::TypeSizeUnavailable(name.to_owned()));
                            }
                        }
                    };
                    if let Err(error) = result {
                        return Err(RuntimeError::from_dispatch(&self.metadata, error)?.into());
                    }
                }
            }
        }
        Ok(())
    }

    /// Check missing type sizes.
    pub fn check_missing_type_sizes(&self) {
        let mut missing = HashSet::new();
        for module in self.metadata.modules_with_events() {
            for event in module.events() {
                for arg in event.arguments() {
                    for primitive in arg.primitives() {
                        if !self.type_sizes.contains_key(&primitive) {
                            missing.insert(format!(
                                "{}::{}::{}",
                                module.name(),
                                event.name,
                                primitive
                            ));
                        }
                    }
                }
            }
        }
        if !missing.is_empty() {
            log::warn!(
                "The following primitive types do not have registered sizes: {:?} \
                If any of these events are received, an error will occur since we cannot decode them",
                missing
            );
        }
    }
}

/// Event subscription simplifies filtering a storage change set stream for
/// events of interest.
pub struct EventSubscription<T: Runtime> {
    subscription: Subscription<StorageChangeSet<T::Hash>>,
    decoder: EventsDecoder<T>,

    // block hash
    block: Option<T::Hash>,
    // extrinsic index.
    extrinsic: Option<usize>,
    // module name and event name
    event: Option<(&'static str, &'static str)>,

    events: VecDeque<RawEvent>,
    finished: bool,
}

impl<T: Runtime> EventSubscription<T> {
    /// Creates a new event subscription.
    pub fn new(
        subscription: Subscription<StorageChangeSet<T::Hash>>,
        decoder: EventsDecoder<T>,
    ) -> Self {
        Self {
            subscription,
            decoder,
            block: None,
            extrinsic: None,
            event: None,
            events: VecDeque::new(),
            finished: false,
        }
    }

    /// Only returns events contained in the block with the given hash.
    pub fn filter_block(&mut self, block: T::Hash) -> &mut Self {
        self.block = Some(block);
        self
    }

    /// Only returns events emitted by extrinsic with index.
    pub fn filter_extrinsic(&mut self, ext_index: usize) -> &mut Self {
        self.extrinsic = Some(ext_index);
        self
    }

    /// Filters events by type.
    pub fn filter_event<E: Event<T>>(&mut self) {
        self.event = Some((E::MODULE, E::EVENT));
    }

    /// Gets the next event.
    pub async fn next(&mut self) -> Option<Result<RawEvent, Error>> {
        loop {
            if let Some(event) = self.events.pop_front() {
                return Some(Ok(event));
            }
            if self.finished {
                return None;
            }

            let change_set = self.subscription.next().await;
            if let Some(hash) = self.block.as_ref() {
                if &change_set.block == hash {
                    self.finished = true;
                } else {
                    continue;
                }
            }

            for (_key, data) in change_set.changes {
                if let Some(data) = data {
                    let raw_events = match self.decoder.decode_events(&mut &data.0[..]) {
                        Ok(events) => events,
                        Err(error) => return Some(Err(error)),
                    };
                    for (phase, raw) in raw_events {
                        if let Phase::ApplyExtrinsic(i) = phase {
                            if let Some(ext_index) = self.extrinsic {
                                if i as usize != ext_index {
                                    continue;
                                }
                            }
                            let event = match raw {
                                Raw::Event(event) => event,
                                Raw::Error(err) => return Some(Err(err.into())),
                            };
                            if let Some((module, variant)) = self.event {
                                if event.module != module || event.variant != variant {
                                    continue;
                                }
                            }
                            self.events.push_back(event);
                        }
                    }
                }
            }
        }
    }
}
