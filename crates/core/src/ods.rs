//! ODS catalogue and workbook exchange.
pub use crate::low_voltage::catalogue::{
    MAX_CELL, MAX_ROWS, apply_catalogue, catalogue_rows, pending_products, placeholder_element, read, transfer_rows, write,
};
pub(crate) use crate::low_voltage::catalogue::{read_workbook, sync_bound, workbook_exists, workbook_path, write_sheets};
