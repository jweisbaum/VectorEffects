//! Registers the pure-Rust blosc decoder with `zarrs`.
//!
//! `zarrs` ships its own blosc codec, but only behind a feature that builds
//! c-blosc and its bundled Snappy through a C++ toolchain. This crate turns
//! that feature off and registers a decode-only replacement under the same
//! Zarr V2 codec id, using `zarrs`' runtime plugin registry.

use std::borrow::Cow;
use std::sync::{Arc, OnceLock};

use zarrs::array::BytesRepresentation;
use zarrs::array::codec::api::{
    ArrayBytesRaw, BytesToBytesCodecTraits, Codec, CodecError, CodecMetadataOptions, CodecOptions,
    CodecRuntimePluginV2, CodecRuntimeRegistryHandleV2, CodecTraits, PartialDecoderCapability,
    PartialEncoderCapability, RecommendedConcurrency, register_codec_v2,
};
use zarrs::metadata::Configuration;
use zarrs::plugin::ZarrVersion;

use crate::blosc;

/// A decode-only `blosc` codec backed by [`crate::blosc`].
#[derive(Clone, Debug)]
pub struct BloscDecodeCodec;

zarrs::plugin::impl_extension_aliases!(BloscDecodeCodec, v2: "blosc");

impl CodecTraits for BloscDecodeCodec {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn configuration(
        &self,
        _version: ZarrVersion,
        _options: &CodecMetadataOptions,
    ) -> Option<Configuration> {
        // This crate never writes Zarr, so there is no configuration to
        // round-trip back into metadata.
        None
    }

    fn partial_decoder_capability(&self) -> PartialDecoderCapability {
        // A blosc container has to be walked from its header, so there is no
        // useful partial read. Each ERA5 chunk is one whole time step anyway.
        PartialDecoderCapability {
            partial_read: false,
            partial_decode: false,
        }
    }

    fn partial_encoder_capability(&self) -> PartialEncoderCapability {
        PartialEncoderCapability {
            partial_encode: false,
        }
    }
}

impl BytesToBytesCodecTraits for BloscDecodeCodec {
    fn into_dyn(self: Arc<Self>) -> Arc<dyn BytesToBytesCodecTraits> {
        self
    }

    fn recommended_concurrency(
        &self,
        _decoded_representation: &BytesRepresentation,
    ) -> Result<RecommendedConcurrency, CodecError> {
        // The export already runs one whole time step per worker thread, so
        // splitting a single chunk further would only add contention.
        Ok(RecommendedConcurrency::new_maximum(1))
    }

    fn encode<'a>(
        &self,
        _decoded_value: ArrayBytesRaw<'a>,
        _options: &CodecOptions,
    ) -> Result<ArrayBytesRaw<'a>, CodecError> {
        Err(CodecError::Other(
            "this blosc codec decodes only; GribHistory never writes Zarr".into(),
        ))
    }

    fn decode<'a>(
        &self,
        encoded_value: ArrayBytesRaw<'a>,
        _decoded_representation: &BytesRepresentation,
        _options: &CodecOptions,
    ) -> Result<ArrayBytesRaw<'a>, CodecError> {
        blosc::decompress(&encoded_value)
            .map(Cow::Owned)
            .map_err(|e| CodecError::Other(e.to_string()))
    }

    fn encoded_representation(
        &self,
        _decoded_representation: &BytesRepresentation,
    ) -> BytesRepresentation {
        BytesRepresentation::UnboundedSize
    }
}

/// Holds the registration alive for the life of the process.
static REGISTRATION: OnceLock<CodecRuntimeRegistryHandleV2> = OnceLock::new();

/// Registers the blosc decoder. Safe to call repeatedly; only the first call
/// registers.
///
/// Every entry point that opens the store calls this, so no caller can forget.
pub fn register() {
    REGISTRATION.get_or_init(|| {
        register_codec_v2(CodecRuntimePluginV2::new(
            |name| name == "blosc",
            |_metadata| Ok(Codec::BytesToBytes(Arc::new(BloscDecodeCodec))),
        ))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registering_twice_is_harmless() {
        register();
        register();
    }

    /// Encoding must fail loudly rather than write a chunk the decoder would
    /// then be asked to read back.
    #[test]
    fn encoding_is_refused() {
        let codec = BloscDecodeCodec;
        let err = codec
            .encode(Cow::Borrowed(&[0u8; 4]), &CodecOptions::default())
            .expect_err("must refuse");
        assert!(format!("{err}").contains("decodes only"), "{err}");
    }
}
