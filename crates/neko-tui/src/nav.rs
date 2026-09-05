//! Navigation and the wallet-facing screens' state.
//!
//! A `Vec<Screen>` stack rather than a flat enum with back-pointers: Esc is one
//! `pop()`, and per-screen state (selection, scroll, half-typed input) survives
//! navigation for free.

use neko_core::WalletView;
use zeroize::Zeroizing;

use crate::input::Field;

/// The chains the wallet knows, taken from `neko-core` rather than declared
/// again here. Two lists would drift, and the one that mattered would be
/// whichever the send screen happened to read.
pub use neko_core::ChainId as Chain;
pub use neko_core::CHAINS;

/// Which form the wallet list has open, if any.
pub enum WalletForm {
    New {
        label: Field,
        words: usize,
    },
    ImportMnemonic {
        label: Field,
        phrase: Field,
        passphrase: Field,
        focus: u8,
    },
    ImportPrivkey {
        label: Field,
        hex: Field,
        focus: u8,
    },
    Rename {
        id: i64,
        label: Field,
    },
    /// Deleting destroys the only copy of the words, so confirmation is typed,
    /// not a keypress.
    Delete {
        id: i64,
        name: String,
        typed: Field,
    },
}

impl WalletForm {
    pub fn title(&self) -> &'static str {
        match self {
            WalletForm::New { .. } => "new wallet",
            WalletForm::ImportMnemonic { .. } => "import recovery phrase",
            WalletForm::ImportPrivkey { .. } => "import private key",
            WalletForm::Rename { .. } => "rename wallet",
            WalletForm::Delete { .. } => "delete wallet",
        }
    }
}

/// Revealing a recovery phrase is the most dangerous screen in the product, so
/// it is a small state machine rather than a flag.
pub enum RevealStage {
    /// Shows what we can and cannot protect against, and takes the password.
    Gate { password: Field, checking: bool },
    /// One word at a time. A screenshot or `capture-pane` leaks 1 word of 12,
    /// not all of them.
    Words {
        words: Vec<String>,
        cursor: usize,
        show_all: bool,
        hide_at: std::time::Instant,
    },
}

pub struct WalletsState {
    pub items: Vec<WalletView>,
    pub selected: usize,
    pub form: Option<WalletForm>,
    pub error: Option<String>,
}

impl WalletsState {
    pub fn new(items: Vec<WalletView>) -> Self {
        Self {
            items,
            selected: 0,
            form: None,
            error: None,
        }
    }
    pub fn current(&self) -> Option<&WalletView> {
        self.items.get(self.selected)
    }
    pub fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let n = self.items.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
    }
}

/// Held only while the reveal screen is open; zeroized on exit.
pub type Phrase = Zeroizing<String>;

/// Settings rows, in display order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingRow {
    Language,
    ApiKey,
    BscApiKey,
    NodeUrl,
    SolanaRpc,
    BitcoinApi,
    EthRpc,
    TonApi,
    AutoLock,
    BorderStyle,
}

pub const SETTING_ROWS: [SettingRow; 10] = [
    SettingRow::Language,
    SettingRow::ApiKey,
    SettingRow::BscApiKey,
    SettingRow::NodeUrl,
    SettingRow::SolanaRpc,
    SettingRow::BitcoinApi,
    SettingRow::EthRpc,
    SettingRow::TonApi,
    SettingRow::AutoLock,
    SettingRow::BorderStyle,
];

impl SettingRow {
    pub fn label(self) -> &'static str {
        match self {
            SettingRow::Language => neko_i18n::t(neko_i18n::Key::Settings_Language),
            SettingRow::ApiKey => neko_i18n::t(neko_i18n::Key::Settings_ApiKey),
            SettingRow::BscApiKey => neko_i18n::t(neko_i18n::Key::Settings_BscApiKey),
            SettingRow::NodeUrl => neko_i18n::t(neko_i18n::Key::Settings_NodeUrl),
            SettingRow::SolanaRpc => neko_i18n::t(neko_i18n::Key::Settings_SolanaRpc),
            SettingRow::BitcoinApi => neko_i18n::t(neko_i18n::Key::Settings_BitcoinApi),
            SettingRow::EthRpc => neko_i18n::t(neko_i18n::Key::Settings_EthRpc),
            SettingRow::TonApi => neko_i18n::t(neko_i18n::Key::Settings_TonApi),
            SettingRow::AutoLock => neko_i18n::t(neko_i18n::Key::Settings_Autolock),
            SettingRow::BorderStyle => neko_i18n::t(neko_i18n::Key::Settings_Border),
        }
    }
}

pub struct SettingsState {
    pub selected: usize,
    /// Set while a text row is being edited.
    pub editing: Option<Field>,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self::new()
    }
}

impl SettingsState {
    pub fn new() -> Self {
        Self {
            selected: 0,
            editing: None,
        }
    }
    pub fn row(&self) -> SettingRow {
        SETTING_ROWS[self.selected.min(SETTING_ROWS.len() - 1)]
    }
    pub fn move_by(&mut self, delta: isize) {
        let n = SETTING_ROWS.len() as isize;
        self.selected = (((self.selected as isize + delta) % n + n) % n) as usize;
    }
}

/// Rows per screen when there is room. Tall terminals stop here rather than
/// growing without bound; short ones get fewer.
pub const DEFAULT_PAGE: usize = 20;
/// Never show fewer than this, even in a cramped window.
pub const MIN_PAGE: usize = 5;
/// Lines the history screen spends on chrome: borders, the address header, the
/// column header, and the selected-row detail block underneath the table.
const HISTORY_CHROME: usize = 13;

/// How many history rows fit in a terminal `height` rows tall.
pub fn history_page_for(height: u16) -> usize {
    (height as usize)
        .saturating_sub(HISTORY_CHROME)
        .clamp(MIN_PAGE, DEFAULT_PAGE)
}

pub struct HistoryState {
    pub chain: Chain,
    pub address: String,
    /// Everything the chain returned, dust included.
    pub entries: Option<Vec<neko_tron::HistoryEntry>>,
    /// Address-poisoning dust is hidden by default: its only purpose is to sit
    /// in this list until you copy the wrong address out of it.
    pub show_dust: bool,
    pub error: Option<String>,
    pub selected: usize,
    /// Rows visible at once. Driven by the real terminal height so paging
    /// matches what is actually on screen.
    pub page: usize,
    pub offset: usize,
}

impl HistoryState {
    pub fn new(chain: Chain, address: String) -> Self {
        Self {
            chain,
            address,
            entries: None,
            show_dust: false,
            error: None,
            selected: 0,
            page: DEFAULT_PAGE,
            offset: 0,
        }
    }

    /// The rows actually on screen, after the dust filter.
    pub fn visible(&self) -> Vec<&neko_tron::HistoryEntry> {
        match &self.entries {
            None => Vec::new(),
            Some(all) => all
                .iter()
                .filter(|e| self.show_dust || !e.is_dust())
                .collect(),
        }
    }

    pub fn dust_count(&self) -> usize {
        self.entries
            .as_ref()
            .map(|all| all.iter().filter(|e| e.is_dust()).count())
            .unwrap_or(0)
    }

    pub fn len(&self) -> usize {
        self.visible().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn move_by(&mut self, delta: isize) {
        let n = self.len();
        if n == 0 {
            return;
        }
        self.selected = (self.selected as isize + delta).clamp(0, n as isize - 1) as usize;
        // Keep the cursor inside the visible window.
        if self.selected < self.offset {
            self.offset = self.selected;
        } else if self.selected >= self.offset + self.page {
            self.offset = self.selected + 1 - self.page;
        }
    }

    pub fn current(&self) -> Option<&neko_tron::HistoryEntry> {
        self.visible().get(self.selected).copied()
    }

    /// Toggling the filter changes how many rows exist, so the cursor has to be
    /// brought back in range.
    pub fn toggle_dust(&mut self) {
        self.show_dust = !self.show_dust;
        let n = self.len();
        self.selected = self.selected.min(n.saturating_sub(1));
        self.offset = self.offset.min(self.selected);
    }
}
