//! UI translation keys for the TUI.
//!
//! This module is intentionally pure data: player names, team abbreviations,
//! and league names are not represented here and should remain English data.

use crate::{i18n_en, i18n_zh};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    En,
    Zh,
}

impl Lang {
    pub fn from_setting(value: &str) -> Option<Self> {
        match value {
            "en" | "EN" | "English" | "english" => Some(Self::En),
            "zh" | "ZH" | "中文" | "Chinese" | "chinese" => Some(Self::Zh),
            _ => None,
        }
    }

    pub fn as_setting(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum T {
    AppName,
    LanguageEnglish,
    LanguageChinese,

    MenuHome,
    MenuRoster,
    MenuRotation,
    MenuTrades,
    MenuDraft,
    MenuFinance,
    MenuInbox,
    MenuCalendar,

    LaunchContinue,
    LaunchNewGame,
    LaunchLoadGame,
    LaunchSettings,
    LaunchQuit,
    LaunchLastSave,
    LaunchNoSave,

    SettingsTitle,
    SettingsLanguage,
    SettingsGodMode,
    SettingsOn,
    SettingsOff,
    SettingsSaved,

    CommonNavigate,
    CommonMove,
    CommonOpen,
    CommonBack,
    CommonConfirm,
    CommonCancel,
    CommonSubmit,
    CommonSave,
    CommonLoad,
    CommonQuit,
    CommonHelp,
    CommonYes,
    CommonNo,
    CommonDelete,
    CommonExport,
    CommonContinue,
    CommonDismiss,
    CommonActions,
    CommonDetail,
    CommonTabs,
    CommonSort,
    CommonPick,
    CommonAuto,
    CommonClear,
    CommonFilter,
    CommonReady,
    CommonError,
    CommonNoSaveLoaded,

    ModalQuitTitle,
    ModalConfirmTitle,
    ModalHelpTitle,
    ModalTradeVerdictTitle,
    ModalExtendContractTitle,
    ModalDraftPickTitle,
    ModalAutoDraftTitle,

    HomeTitle,
    HomeRecord,
    HomeConferenceRank,
    HomeConferenceStandings,
    HomeTeamLeaders,
    HomeLeagueLeaders,
    HomeTeamStats,
    HomeFinances,
    HomeStartingLineup,
    HomeStatPoints,
    HomeStatAllowed,
    HomeStatRebounds,
    HomeStatAssists,
    FinanceAvgAttendance,
    FinanceRevenueYTD,
    FinanceProfitYTD,
    FinanceCash,

    InboxTitle,
    InboxMessages,
    InboxTradeDemands,
    InboxNews,
    InboxNoMessages,
    InboxNoDemands,
    InboxNoNews,

    SimDay,
    SimWeek,
    SimMonth,
    SimTradeDeadline,
    SimSeasonAdvance,
    SimAnimating,
    SimSkip,

    RosterTitle,
    RosterMyRoster,
    RosterFreeAgents,
    RosterPlayer,
    RosterPosition,
    RosterOverall,
    RosterPotential,
    RosterAge,
    RosterSalary,
    RosterRole,
    RosterMorale,
    RosterSortLabel,
    RosterSortOverall,
    RosterSortPoints,
    RosterSortPosition,
    RosterTrain,
    RosterExtend,
    RosterCut,
    RosterSetRole,
    RosterNoPlayers,
    RosterTrainingFocus,
    RosterRoleHeader,

    RoleStar,
    RoleStarter,
    RoleSixthMan,
    RoleRolePlayer,
    RoleBenchWarmer,
    RoleProspect,

    RotationTitle,
    RotationStarters,
    RotationBench,
    RotationSlot,
    RotationMinutes,
    RotationClearSlot,
    RotationClearAll,

    TradesTitle,
    TradesInbox,
    TradesMyProposals,
    TradesBuilder,
    TradesRumors,
    TradesAccept,
    TradesReject,
    TradesCounter,
    TradesActionPickerTitle,
    TradesPropose,
    TradesYouSend,
    TradesSubmit,
    TradesIncomingOffersNone,
    TradesNoProposals,
    TradesNoRumors,
    TradesPickBothSides,
    TradesToggleTeamMode,
    TradesSwapIncomingTeam,
    TradesInsufficientValue,
    TradesPickTeamTitle,
    TradesRosterPreview,
    TradesPayrollCap,
    TradesBuilderTitle,
    TradesBuilderTopBar,
    TradesTargetTeam,
    TradesMyTeam,
    TradesSendList,
    TradesReceiveList,
    TradesSectionPlayers,
    TradesSectionPicks,
    TradesPicksDeferred,
    TradesVerdictTitle,
    TradesVerdictSent,
    TradesVerdictReceived,
    TradesVerdictDelta,
    TradesVerdictPrompt,
    TradesWarnSalaryMatch,
    TradesWarnHardCap,
    TradesWarnNTC,
    TradesNoteTradeKicker,
    TradesWarnRosterSize,
    TradesForceTradeChip,
    TradesGmAccept,
    TradesGmCounterMild,
    TradesGmCounterDemand,
    TradesGmRejectInsufficient,
    TradesGmRejectCba,
    TradesGmRejectSalaryMatch,
    TradesGmRejectHardCap,
    TradesGmRejectRoster,
    TradesGmRejectUntouchable,
    TradesGmRejectBadFaith,
    TradesGodAcceptDialog,
    TradesVerdictCapPass,

    DraftTitle,
    DraftBoard,
    DraftOrder,
    DraftScout,
    DraftAutoPick,
    DraftProspect,
    DraftProjectedPick,
    DraftNoProspect,
    DraftNotActive,
    DraftUnknownTeam,

    FinanceTitle,
    FinancePayroll,
    FinanceCap,
    FinanceTax,
    FinanceApron1,
    FinanceApron2,
    FinanceContracts,
    FinanceExtensions,
    FinanceYears,
    FinanceTotal,
    FinanceSortTotal,
    FinanceSortYears,

    CalendarTitle,
    CalendarSchedule,
    CalendarStandings,
    CalendarPlayoffs,
    CalendarAwards,
    CalendarAllStar,
    CalendarCup,
    CalendarDayOf,
    CalendarNoSchedule,
    CalDayMon,
    CalDayTue,
    CalDayWed,
    CalDayThu,
    CalDayFri,
    CalDaySat,
    CalDaySun,
    CalMonJan,
    CalMonFeb,
    CalMonMar,
    CalMonApr,
    CalMonMay,
    CalMonJun,
    CalMonJul,
    CalMonAug,
    CalMonSep,
    CalMonOct,
    CalMonNov,
    CalMonDec,

    SavesTitle,
    SavesLoad,
    SavesNew,
    SavesDelete,
    SavesExport,
    SavesNoSaves,
    SavesSaveWritten,

    NewGameTitle,
    NewGameSavePath,
    NewGameTeam,
    NewGameMode,
    NewGameSeason,
    NewGameSeed,
    NewGameConfirm,
}

pub fn t(lang: Lang, key: T) -> &'static str {
    match lang {
        Lang::En => i18n_en::lookup(key),
        Lang::Zh => i18n_zh::lookup(key),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lookup_preserves_english_menu_labels() {
        assert_eq!(t(Lang::En, T::MenuHome), "Home");
        assert_eq!(t(Lang::En, T::MenuRoster), "Roster");
        assert_eq!(t(Lang::En, T::ModalQuitTitle), "Quit nba3k?");
    }

    #[test]
    fn lookup_returns_chinese_for_core_navigation() {
        assert_eq!(t(Lang::Zh, T::MenuHome), "主页");
        assert_eq!(t(Lang::Zh, T::MenuRoster), "阵容");
        assert_eq!(t(Lang::Zh, T::CommonCancel), "取消");
    }

    #[test]
    fn lang_setting_roundtrips() {
        assert_eq!(Lang::from_setting("en"), Some(Lang::En));
        assert_eq!(Lang::from_setting("zh"), Some(Lang::Zh));
        assert_eq!(Lang::Zh.as_setting(), "zh");
        assert_eq!(Lang::from_setting("fr"), None);
    }
}
