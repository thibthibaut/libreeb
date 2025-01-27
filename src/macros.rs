#[macro_export]
macro_rules! extract_bits {
    ($value:expr, $type:ty, $start:expr, $len:expr) => {{
        // Create a mask with 1s in the range we want to extract
        let mask = ((1u16 << $len) - 1) << $start;
        // Extract the bits and shift them right to start position
        let extracted = ($value & mask) >> $start;
        // Cast to the desired type
        extracted as $type
    }};
}

#[macro_export]
macro_rules! define_raw_evt {
    // Pattern for the input
    (
        #[storage($storage:ty), discriminant($discriminant_start:expr, $discriminant_len:expr)]
        enum $enum_name:ident {
            $(
                $variant:ident ( $event_type:literal ) {
                    $(
                        #[$field_start:expr,$field_len:expr]
                        $field:ident : $type:ty
                    ),*
                }
            ),*
        }
    ) => {
        // Define the enum and its variants
        #[derive(Debug)]
        enum $enum_name {
            $(
                $variant { $($field: $type),* },
            )*
            Unknown, // Add an unknown variant
        }
        // Implement from for the event data type
        impl From<&[u8]> for $enum_name {
            fn from(bytes: &[u8]) -> Self {
                let bytes: [u8; 2] = bytes.try_into().unwrap();
                let value = u16::from_le_bytes(bytes);
                let event_type = $crate::extract_bits!(value, u16, $discriminant_start, $discriminant_len);
                match event_type {
                    $(
                        $event_type => {
                            $(
                                let $field = $crate::extract_bits!(value, $type, $field_start, $field_len);
                            )*
                            Self::$variant {
                                $($field),*
                            }
                        }
                    ),*
                    _ => Self::Unknown,
                }
            }
        }
    };
}
