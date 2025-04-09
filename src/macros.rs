#[macro_export]
macro_rules! extract_bits {
    ($value:expr, $source_type:ty, $target_type:ty, $start:expr, $len:expr) => {{
        // Create a mask with 1s in the range we want to extract
        let one: $source_type = 1;
        let mask: $source_type = ((one << $len) - 1) << $start;
        // Extract the bits and shift them right to start position
        let extracted = ($value & mask) >> $start;
        // Cast to the desired type
        extracted as $target_type
    }};
}

#[macro_export]
macro_rules! define_raw_evt {
    // Pattern for the input
    (
        #[storage($storage:ty), size($size:literal), discriminant($discriminant_start:expr, $discriminant_len:expr)]
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
                let bytes: [u8; $size] = bytes.try_into().unwrap();
                let value = <$storage>::from_le_bytes(bytes);
                let event_type = $crate::extract_bits!(value, $storage, u16, $discriminant_start, $discriminant_len);
                match event_type {
                    $(
                        $event_type => {
                            $(
                                let $field = $crate::extract_bits!(value,$storage, $type, $field_start, $field_len);
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
