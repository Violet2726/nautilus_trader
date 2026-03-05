// -------------------------------------------------------------------------------------------------
//  Copyright (C) 2015-2026 Nautech Systems Pty Ltd. All rights reserved.
//  https://nautechsystems.io
//
//  Licensed under the GNU Lesser General Public License Version 3.0 (the "License");
//  You may not use this file except in compliance with the License.
//  You may obtain a copy of the License at https://www.gnu.org/licenses/lgpl-3.0.en.html
//
//  Unless required by applicable law or agreed to in writing, software
//  distributed under the License is distributed on an "AS IS" BASIS,
//  WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
//  See the License for the specific language governing permissions and
//  limitations under the License.
// -------------------------------------------------------------------------------------------------

use std::{collections::HashMap, str::FromStr, sync::Arc};

use arrow::{
    array::{Array, BooleanArray, StringBuilder, StringArray, UInt16Array, UInt64Array},
    datatypes::{DataType, Field, Schema},
    error::ArrowError,
    record_batch::RecordBatch,
};
use nautilus_model::{
    data::status::InstrumentStatus,
    enums::{FromU16, MarketStatusAction},
    identifiers::InstrumentId,
};
use ustr::Ustr;

use super::{DecodeDataFromRecordBatch, EncodingError, KEY_INSTRUMENT_ID, extract_column};
use crate::arrow::{ArrowSchemaProvider, Data, DecodeFromRecordBatch, EncodeToRecordBatch};

impl ArrowSchemaProvider for InstrumentStatus {
    fn get_schema(metadata: Option<HashMap<String, String>>) -> Schema {
        let fields = vec![
            Field::new("action", DataType::UInt16, false),
            Field::new("ts_event", DataType::UInt64, false),
            Field::new("ts_init", DataType::UInt64, false),
            Field::new("reason", DataType::Utf8, true),
            Field::new("trading_event", DataType::Utf8, true),
            Field::new("is_trading", DataType::Boolean, true),
            Field::new("is_quoting", DataType::Boolean, true),
            Field::new("is_short_sell_restricted", DataType::Boolean, true),
        ];

        match metadata {
            Some(metadata) => Schema::new_with_metadata(fields, metadata),
            None => Schema::new(fields),
        }
    }
}

fn parse_metadata(metadata: &HashMap<String, String>) -> Result<InstrumentId, EncodingError> {
    let instrument_id_str = metadata
        .get(KEY_INSTRUMENT_ID)
        .ok_or_else(|| EncodingError::MissingMetadata(KEY_INSTRUMENT_ID))?;
    let instrument_id = InstrumentId::from_str(instrument_id_str)
        .map_err(|e| EncodingError::ParseError(KEY_INSTRUMENT_ID, e.to_string()))?;

    Ok(instrument_id)
}

impl EncodeToRecordBatch for InstrumentStatus {
    fn encode_batch(
        metadata: &HashMap<String, String>,
        data: &[Self],
    ) -> Result<RecordBatch, ArrowError> {
        let mut action_builder = UInt16Array::builder(data.len());
        let mut ts_event_builder = UInt64Array::builder(data.len());
        let mut ts_init_builder = UInt64Array::builder(data.len());
        let mut reason_builder = StringBuilder::new();
        let mut trading_event_builder = StringBuilder::new();
        let mut is_trading_builder = BooleanArray::builder(data.len());
        let mut is_quoting_builder = BooleanArray::builder(data.len());
        let mut is_short_sell_restricted_builder = BooleanArray::builder(data.len());

        for item in data {
            action_builder.append_value(item.action as u16);
            ts_event_builder.append_value(item.ts_event.as_u64());
            ts_init_builder.append_value(item.ts_init.as_u64());
            match item.reason {
                Some(ref r) => reason_builder.append_value(r.as_str()),
                None => reason_builder.append_null(),
            }
            match item.trading_event {
                Some(ref r) => trading_event_builder.append_value(r.as_str()),
                None => trading_event_builder.append_null(),
            }
            match item.is_trading {
                Some(v) => is_trading_builder.append_value(v),
                None => is_trading_builder.append_null(),
            }
            match item.is_quoting {
                Some(v) => is_quoting_builder.append_value(v),
                None => is_quoting_builder.append_null(),
            }
            match item.is_short_sell_restricted {
                Some(v) => is_short_sell_restricted_builder.append_value(v),
                None => is_short_sell_restricted_builder.append_null(),
            }
        }

        RecordBatch::try_new(
            Self::get_schema(Some(metadata.clone())).into(),
            vec![
                Arc::new(action_builder.finish()),
                Arc::new(ts_event_builder.finish()),
                Arc::new(ts_init_builder.finish()),
                Arc::new(reason_builder.finish()),
                Arc::new(trading_event_builder.finish()),
                Arc::new(is_trading_builder.finish()),
                Arc::new(is_quoting_builder.finish()),
                Arc::new(is_short_sell_restricted_builder.finish()),
            ],
        )
    }

    fn metadata(&self) -> HashMap<String, String> {
        let mut metadata = HashMap::new();
        metadata.insert(
            KEY_INSTRUMENT_ID.to_string(),
            self.instrument_id.to_string(),
        );
        metadata
    }
}

impl DecodeFromRecordBatch for InstrumentStatus {
    fn decode_batch(
        metadata: &HashMap<String, String>,
        record_batch: RecordBatch,
    ) -> Result<Vec<Self>, EncodingError> {
        let instrument_id = parse_metadata(metadata)?;
        let cols = record_batch.columns();

        let action_values = extract_column::<UInt16Array>(cols, "action", 0, DataType::UInt16)?;
        let ts_event_values = extract_column::<UInt64Array>(cols, "ts_event", 1, DataType::UInt64)?;
        let ts_init_values = extract_column::<UInt64Array>(cols, "ts_init", 2, DataType::UInt64)?;
        let reason_values = extract_column::<StringArray>(cols, "reason", 3, DataType::Utf8)?;
        let trading_event_values =
            extract_column::<StringArray>(cols, "trading_event", 4, DataType::Utf8)?;
        let is_trading_values =
            extract_column::<BooleanArray>(cols, "is_trading", 5, DataType::Boolean)?;
        let is_quoting_values =
            extract_column::<BooleanArray>(cols, "is_quoting", 6, DataType::Boolean)?;
        let is_short_sell_restricted_values =
            extract_column::<BooleanArray>(cols, "is_short_sell_restricted", 7, DataType::Boolean)?;

        let result: Vec<Self> = (0..record_batch.num_rows())
            .map(|row| {
                let action_value = action_values.value(row);
                let action =
                    MarketStatusAction::from_u16(action_value).unwrap_or(MarketStatusAction::None);

                let reason = if reason_values.is_null(row) {
                    None
                } else {
                    Some(Ustr::from(reason_values.value(row)))
                };

                let trading_event = if trading_event_values.is_null(row) {
                    None
                } else {
                    Some(Ustr::from(trading_event_values.value(row)))
                };

                let is_trading = if is_trading_values.is_null(row) {
                    None
                } else {
                    Some(is_trading_values.value(row))
                };

                let is_quoting = if is_quoting_values.is_null(row) {
                    None
                } else {
                    Some(is_quoting_values.value(row))
                };

                let is_short_sell_restricted = if is_short_sell_restricted_values.is_null(row) {
                    None
                } else {
                    Some(is_short_sell_restricted_values.value(row))
                };

                Self {
                    instrument_id,
                    action,
                    ts_event: ts_event_values.value(row).into(),
                    ts_init: ts_init_values.value(row).into(),
                    reason,
                    trading_event,
                    is_trading,
                    is_quoting,
                    is_short_sell_restricted,
                }
            })
            .collect();

        Ok(result)
    }
}

impl DecodeDataFromRecordBatch for InstrumentStatus {
    fn decode_data_batch(
        metadata: &HashMap<String, String>,
        record_batch: RecordBatch,
    ) -> Result<Vec<Data>, EncodingError> {
        let items: Vec<Self> = Self::decode_batch(metadata, record_batch)?;
        Ok(items.into_iter().map(Data::from).collect())
    }
}
