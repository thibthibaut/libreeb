//! Event format decoders.
//!
//! This module contains decoders for different Prophesee event formats:
//! - [`Evt3`]/[`Evt3Decoder`]: Native Gen4 format (most common)
//! - [`Evt21`]/[`Evt21Decoder`]: EVT2.1 with mask-based encoding
//! - [`Evt2`]/[`Evt2Decoder`]: Legacy EVT2 format
//!
//! Each decoder implements the internal [`RawFormatDecoder`] trait and
//! is wrapped by the public [`EventDecoder`] API.
//!
//! ## Format Specifications
//!
//! - **EVT3**: https://docs.prophesee.ai/stable/data/encoding_formats/evt3.html
//! - **EVT2.1**: https://docs.prophesee.ai/stable/data/encoding_formats/evt21.html

mod evt2;
mod evt2_1;
mod evt3;

pub use evt2::{Evt2, Evt2Decoder};
pub use evt2_1::{Evt21, Evt21Decoder};
pub use evt3::{Evt3, Evt3Decoder};
