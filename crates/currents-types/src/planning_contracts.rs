//! C12 portable planning data. No calendar expansion, forecasting or recognition algorithms.
use crate::{schema::Schema, *};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A Gregorian civil date, never an inferred midnight or a timestamp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CivilDateV1(pub Date);
impl Serialize for CivilDateV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0.to_string())
    }
}
impl<'de> Deserialize<'de> for CivilDateV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        let format = ::time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
            .map_err(serde::de::Error::custom)?;
        Date::parse(&text, &format)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}
impl Schema for CivilDateV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"string","format":"date"})
    }
}
/// Product horizons are calendar months, not arbitrary durations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForecastMonthsV1 {
    One,
    Three,
    Twelve,
}
impl Serialize for ForecastMonthsV1 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_u8(match self {
            Self::One => 1,
            Self::Three => 3,
            Self::Twelve => 12,
        })
    }
}
impl<'de> Deserialize<'de> for ForecastMonthsV1 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match u8::deserialize(deserializer)? {
            1 => Ok(Self::One),
            3 => Ok(Self::Three),
            12 => Ok(Self::Twelve),
            _ => Err(serde::de::Error::custom("unsupported forecast horizon")),
        }
    }
}
impl Schema for ForecastMonthsV1 {
    fn schema() -> serde_json::Value {
        serde_json::json!({"type":"integer","enum":[1,3,12]})
    }
}
dto!(PlanningEmptyV1 {});
dto!(PlanningAmountsV1 { nominal:SourceValue<Amount>, low:SourceValue<Amount>, base:SourceValue<Amount>, high:SourceValue<Amount> });
dto!(SettlementRangeV1 {
    earliest: CivilDateV1,
    latest: CivilDateV1
});
tagged!(SettlementRuleV1 { SameDay(PlanningEmptyV1), NextWeekday(PlanningEmptyV1), PreviousWeekday(PlanningEmptyV1), HolidayCalendar(PolicyReferenceV1), Range(SettlementRangeV1) });
choices!(CommitmentKindV1 {
    Expense,
    Income,
    Transfer,
    CardPurchase,
    CardPayment,
    DebtPayment
});
choices!(RecurrenceCadenceV1 {
    Once,
    Daily,
    Weekly,
    Monthly,
    Annual
});
tagged!(RecurrenceDayV1 { Day(u8), LastDay(PlanningEmptyV1) });
dto!(RecurrenceV1 { cadence:RecurrenceCadenceV1, interval:u32, month:Option<u8>, day:Option<RecurrenceDayV1> });
dto!(PlannedLegV1 {
    id: RecordId,
    account_id: AccountId,
    amounts: PlanningAmountsV1,
    settlement: SettlementRuleV1
});
choices!(PlanningExceptionActionV1 {
    Skip,
    Cancel,
    Replace
});
dto!(PlanningExceptionV1 { ordinal:CommitSeq, action:PlanningExceptionActionV1, due_date:Option<CivilDateV1>, legs:Vec<PlannedLegV1>, reason:String });
dto!(CommitmentV1 { id:CommitmentId, version:u32, kind:CommitmentKindV1, amounts:PlanningAmountsV1, legs:Vec<PlannedLegV1>, due_date:CivilDateV1, settlement:SettlementRuleV1, recurrence:RecurrenceV1, recurrence_version:u32, timezone:String, timezone_data_version:String, calendar_version:u32, start:CivilDateV1, end:Option<CivilDateV1>, exceptions:Vec<PlanningExceptionV1>, provenance:InterpretationV1 });
dto!(OccurrenceKeyV1 {
    commitment_id: CommitmentId,
    series_generation: u32,
    ordinal: CommitSeq
});
dto!(OccurrenceV1 { key:OccurrenceKeyV1, version:u32, legs:Vec<PlannedLegV1>, due_date:CivilDateV1, settlement_range:SettlementRangeV1 });
choices!(StartingInclusionV1 {
    IncludedInStartingBalance,
    AfterStartingBoundary,
    Unknown
});
dto!(OccurrenceMatchV1 {
    occurrence: OccurrenceKeyV1,
    planned_leg: RecordId,
    economic_leg: LegId,
    allocation_version: u32,
    amount: Amount,
    inclusion: StartingInclusionV1,
    pending: bool
});
dto!(CommitmentVersionV1 {
    id: CommitmentId,
    version: u32
});
dto!(ForecastRequestV1 { basis:ReadBasis, start:CivilDateV1, months:ForecastMonthsV1, commitment_versions:Vec<CommitmentVersionV1>, assumption_versions:Vec<PolicyReferenceV1>, scenario:RecordId });
dto!(ForecastAssumptionV1 { id:RecordId, version:u32, account:AccountId, category:Option<RecordId>, dates:DateScope, exclusions:Vec<LegId>, amounts:PlanningAmountsV1, provenance:InterpretationV1 });
dto!(ForecastStartingPositionV1 { account:AccountId, as_of:CivilDateV1, amount:SourceValue<Amount>, selected_basis:Vec<RecordId>, inclusion_map:Vec<OccurrenceMatchV1> });
dto!(ExpandedForecastInputsV1 { commitments:Vec<CommitmentV1>, occurrences:Vec<OccurrenceV1>, assumptions:Vec<ForecastAssumptionV1>, starting_positions:Vec<ForecastStartingPositionV1>, matches:Vec<OccurrenceMatchV1> });
// Referenced timezone bytes are retained evidence, not an ambient host timezone database.
dto!(ForecastCalendarDataV1 { timezone:String, timezone_data_version:String, timezone_data:ContentReferenceV1, calendar_version:u32, holidays:Vec<CivilDateV1> });
dto!(ForecastDailyBalanceV1 { account:AccountId, date:CivilDateV1, amount:SourceValue<Amount> });
dto!(ForecastMinimumV1 { account:AccountId, date:SourceValue<CivilDateV1>, amount:SourceValue<Amount> });
choices!(ForecastVariantV1 {
    Base,
    Downside,
    Upside,
    OwnerDefined
});
dto!(ForecastOutputV1 { variant:ForecastVariantV1, daily_balances:Vec<ForecastDailyBalanceV1>, minimum_balances:Vec<ForecastMinimumV1>, coverage:Vec<CoverageV1>, unknown_accounts:Vec<AccountId>, omitted_accounts:Vec<AccountId>, starting_position_uncertainty:Vec<String>, inclusion_maps:Vec<OccurrenceMatchV1>, assumptions:Vec<String>, matching_conflicts:Vec<RecordId> });
dto!(ForecastRunV1 { id:RecordId, request:ForecastRequestV1, expanded_inputs:ExpandedForecastInputsV1, calendar_data:ForecastCalendarDataV1, algorithm_version:u32, outputs:Vec<ForecastOutputV1> });
