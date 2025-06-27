#[macro_export]
macro_rules! declare_raw_evt {
    (
        $vis:vis struct $name:ident($data_ty:ty);
        $($field:ident($ret_ty:ty): $high:literal, $low:literal;)+
    ) => {
        #[derive(FromBytes, Immutable, KnownLayout, Copy, Clone)]
        #[repr(C)]
        $vis struct $name {
            data: $data_ty,
        }

        impl $name {
            $(
                /// Extracts bits $high:$low from the raw data
                /// The extraction works by:
                /// 1. Right-shifting by $low positions to align the field to bit 0
                /// 2. Masking with ((1 << field_width) - 1) to isolate the field bits
                /// 3. Casting to the target return type
                fn $field(&self) -> $ret_ty {
                    ((self.data >> $low) & ((1 << ($high - $low + 1)) - 1)) as $ret_ty
                }
            )+
        }
    };
}
