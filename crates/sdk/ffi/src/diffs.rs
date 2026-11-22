//! Complete 11-variant `eyeball_im::VectorDiff` translation across `UniFFI` boundaries.
//! Every index check validates against Kotlin's signed 32-bit `List` index ceiling (`i32::MAX`).

use eyeball_im::VectorDiff;

use crate::errors::FfiClientError;

/// Translates a batch of SDK `VectorDiff<S>` items using `map_diff` into bounded FFI diffs.
///
/// # Errors
/// Returns [`FfiClientError::IndexOutOfRange`] if any diff index exceeds Kotlin's `Int` bounds.
pub(crate) fn translate_diffs<S, F, T>(
    diffs: Vec<VectorDiff<S>>,
    mut map_diff: F,
) -> Result<Vec<T>, FfiClientError>
where
    F: FnMut(VectorDiff<S>) -> Result<T, FfiClientError>,
{
    diffs.into_iter().map(&mut map_diff).collect()
}

/// Validates and maps a single `eyeball_im::VectorDiff<S>` into any concrete 11-variant FFI diff.
#[macro_export]
macro_rules! define_ffi_vector_diff {
    ($doc:literal, $name:ident, $item:ty) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, uniffi::Enum)]
        pub enum $name {
            /// Append all supplied items in order to the end of the list.
            Append {
                /// Items appended to the tail.
                values: Vec<$item>,
            },
            /// Remove all items from the list.
            Clear,
            /// Insert one item at index 0.
            PushFront {
                /// Inserted front item.
                value: $item,
            },
            /// Append one item to the end of the list.
            PushBack {
                /// Appended tail item.
                value: $item,
            },
            /// Remove the first item in the list.
            PopFront,
            /// Remove the last item in the list.
            PopBack,
            /// Insert one item before `index`.
            Insert {
                /// Zero-based destination index checked against Kotlin `Int`.
                index: u32,
                /// Inserted item.
                value: $item,
            },
            /// Replace the item at `index`.
            Set {
                /// Zero-based target index checked against Kotlin `Int`.
                index: u32,
                /// Replacement item.
                value: $item,
            },
            /// Remove the item at `index`.
            Remove {
                /// Zero-based target index checked against Kotlin `Int`.
                index: u32,
            },
            /// Shrink the list to `length` items.
            Truncate {
                /// Retained prefix length checked against Kotlin `Int`.
                length: u32,
            },
            /// Replace the entire list contents atomically.
            Reset {
                /// Complete replacement list in display order.
                values: Vec<$item>,
            },
        }

        impl $name {
            /// Maps a single `eyeball_im::VectorDiff<S>` into this FFI diff enum.
            pub(crate) fn from_sdk<S: Clone>(
                diff: eyeball_im::VectorDiff<S>,
                mut map_item: impl FnMut(S) -> $item,
            ) -> Result<Self, $crate::errors::FfiClientError> {
                Ok(match diff {
                    eyeball_im::VectorDiff::Append { values } => Self::Append {
                        values: values.into_iter().map(map_item).collect(),
                    },
                    eyeball_im::VectorDiff::Clear => Self::Clear,
                    eyeball_im::VectorDiff::PushFront { value } => Self::PushFront {
                        value: map_item(value),
                    },
                    eyeball_im::VectorDiff::PushBack { value } => Self::PushBack {
                        value: map_item(value),
                    },
                    eyeball_im::VectorDiff::PopFront => Self::PopFront,
                    eyeball_im::VectorDiff::PopBack => Self::PopBack,
                    eyeball_im::VectorDiff::Insert { index, value } => Self::Insert {
                        index: $crate::ids::checked_list_index(index)?,
                        value: map_item(value),
                    },
                    eyeball_im::VectorDiff::Set { index, value } => Self::Set {
                        index: $crate::ids::checked_list_index(index)?,
                        value: map_item(value),
                    },
                    eyeball_im::VectorDiff::Remove { index } => Self::Remove {
                        index: $crate::ids::checked_list_index(index)?,
                    },
                    eyeball_im::VectorDiff::Truncate { length } => Self::Truncate {
                        length: $crate::ids::checked_list_index(length)?,
                    },
                    eyeball_im::VectorDiff::Reset { values } => Self::Reset {
                        values: values.into_iter().map(map_item).collect(),
                    },
                })
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    define_ffi_vector_diff!("Test diff.", TestDiff, u32);

    #[test]
    fn all_eleven_vector_diff_variants_and_bounds_translate_cleanly() {
        let cases = vec![
            VectorDiff::Append {
                values: [1_u32, 2].into_iter().collect(),
            },
            VectorDiff::Clear,
            VectorDiff::PushFront { value: 3 },
            VectorDiff::PushBack { value: 4 },
            VectorDiff::PopFront,
            VectorDiff::PopBack,
            VectorDiff::Insert { index: 1, value: 5 },
            VectorDiff::Set { index: 0, value: 6 },
            VectorDiff::Remove { index: 2 },
            VectorDiff::Truncate { length: 1 },
            VectorDiff::Reset {
                values: [7_u32, 8].into_iter().collect(),
            },
        ];
        let translated = translate_diffs(cases, |d| TestDiff::from_sdk(d, |v| v)).unwrap();
        assert_eq!(
            translated,
            vec![
                TestDiff::Append { values: vec![1, 2] },
                TestDiff::Clear,
                TestDiff::PushFront { value: 3 },
                TestDiff::PushBack { value: 4 },
                TestDiff::PopFront,
                TestDiff::PopBack,
                TestDiff::Insert { index: 1, value: 5 },
                TestDiff::Set { index: 0, value: 6 },
                TestDiff::Remove { index: 2 },
                TestDiff::Truncate { length: 1 },
                TestDiff::Reset { values: vec![7, 8] },
            ]
        );
        let overflow = VectorDiff::Insert {
            index: (i32::MAX as usize) + 1,
            value: 9_u32,
        };
        assert!(matches!(
            TestDiff::from_sdk(overflow, |v| v),
            Err(FfiClientError::IndexOutOfRange)
        ));
    }
}
