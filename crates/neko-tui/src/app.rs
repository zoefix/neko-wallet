//! Application state machine.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use neko_core::{CoreError, Session, VaultFile};
use neko_vault::password::{self, Strength, Warning};
use neko_vault::profile;

use crate::event::{AppEvent, ReqId};
use crate::input::Field;
use crate::nav::{Chain, RevealStage, SettingsState, WalletForm, WalletsState};
use crate::theme::BorderStyle;

pub const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
pub const MIN_COLS: u16 = 80;
pub const MIN_ROWS: u16 = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SetupField {
    Email,
    Password,
    Confirm,
}

pub enum Screen {
    /// No vault file yet: create one.
    FirstRun { focus: SetupField, warned: bool },
    /// A vault exists: unlock it.
    Login { email_focused: bool },
    /// Argon2id is grinding in the blocking pool.
    Deriving {
        req: ReqId,
        started: Instant,
        creating: bool,
    },
    /// Wallet list. The root of the unlocked navigation stack.
    Wallets(WalletsState),
    /// Wallet > chain.
    Chains {
        wallet_id: i64,
        name: String,
        selected: usize,
    },
    /// Wallet > chain > assets, with the receiving address.
    Assets {
        wallet_id: i64,
        name: String,
        chain: Chain,
        address: String,
        selected: usize,
    },
    /// Outgoing transfer.
    /// Boxed: this is much the largest screen state, and an unboxed
    /// variant would make every other screen carry its size.
    Send(Box<crate::send::SendState>),
    /// Recovery phrase, behind a re-authentication gate.
    Reveal {
        wallet_id: i64,
        name: String,
        stage: RevealStage,
    },
    /// Transaction history for one address.
    History(crate::nav::HistoryState),
    /// Configuration.
    Settings(crate::nav::SettingsState),
    /// Idle timeout fired.
    Locked { reason: LockReason },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LockReason {
    Idle,
    Manual,
}

/// Transient bottom-of-screen feedback.
pub struct Toast {
    pub text: String,
    pub until: Instant,
}

pub struct App {
    pub nav: Vec<Screen>,
    pub toast: Option<Toast>,
    pub clipboard: crate::clipboard::Clipboard,
    pub db_path: PathBuf,
    pub screen: Screen,
    pub email: Field,
    pub password: Field,
    pub confirm: Field,
    pub session: Option<Session>,
    pub error: Option<String>,
    pub warnings: Vec<Warning>,
    pub border: BorderStyle,
    pub autolock: Duration,
    pub last_input: Instant,
    pub tick: u64,
    pub should_quit: bool,
    /// Which server speaks for mainnet. There is no chain to choose.
    /// TRON's node. `None` means the built-in default.
    pub node_url: Option<String>,
    /// Solana's cluster. `None` means the public one, which rate-limits hard
    /// enough that history loses pages - the reason this is configurable.
    pub solana_rpc: Option<String>,
    /// Bitcoin's Esplora endpoint. `None` means Blockstream's public one.
    ///
    /// Configurable for a sharper reason than the others: a plain Bitcoin node
    /// cannot answer what an address holds, so this server is not an
    /// alternative to asking the chain - it is the only one being asked.
    pub bitcoin_api: Option<String>,
    /// Ethereum's node. `None` means the public one, which rate-limits.
    pub eth_rpc: Option<String>,
    /// Polygon's node. `None` means the public one.
    pub polygon_rpc: Option<String>,
    /// Base's node. `None` means the public one.
    pub base_rpc: Option<String>,
    /// toncenter. `None` means the public endpoint. Configurable for the same
    /// reason as Esplora: reading a TON balance means running a contract's own
    /// method, so this server is not an alternative to asking the chain - it is
    /// the only one being asked.
    pub ton_api: Option<String>,
    pub api_key: Option<String>,
    /// NodeReal key for BNB Chain history. Balances and transfers work without
    /// it; only history needs an indexer.
    pub bsc_api_key: Option<String>,
    /// Etherscan V2 key. Optional, and covers every EVM chain at once: with
    /// one set, it is preferred over NodeReal on all of them.
    pub etherscan_key: Option<String>,
    /// toncenter key. Optional - it raises a rate limit rather than unlocking
    /// anything, and unlike the BNB Chain key it is balances rather than
    /// history that hit that limit.
    pub ton_api_key: Option<String>,
    /// `(symbol, decimals, amount)` in minimal units for the address on screen.
    pub balances: Option<Vec<(String, u8, i128)>>,
    pub balances_req: Option<ReqId>,
    /// Native-coin prices, quoted on-chain. Empty until fetched, and never
    /// guessed: an unknown price withholds a wallet's total rather than
    /// silently leaving that holding out of it.
    pub prices: neko_core::Prices,
    /// Shown on the unlock screen until dismissed. Used to tell the user
    /// that a previous update failed to start and can be rolled back.
    pub startup_warning: Option<String>,
    pub balances_error: Option<String>,
    pub inflight: Option<ReqId>,
    /// Last known terminal size, so list paging matches what is on screen.
    pub viewport: (u16, u16),
    /// Wallet balance fetches still outstanding, for the "refreshing" hint.
    pub assets_pending: usize,
    /// Set when the wallet list needs a background refresh on the next tick.
    pub assets_stale: bool,
    pub locale: neko_i18n::Locale,
    req_seq: u64,
}

impl App {
    pub fn new(db_path: PathBuf) -> Self {
        let exists = db_path.exists();
        Self {
            nav: Vec::new(),
            toast: None,
            clipboard: crate::clipboard::Clipboard::detect(),
            screen: if exists {
                Screen::Login {
                    email_focused: true,
                }
            } else {
                Screen::FirstRun {
                    focus: SetupField::Email,
                    warned: false,
                }
            },
            db_path,
            email: Field::new(false),
            password: Field::new(true),
            confirm: Field::new(true),
            session: None,
            error: None,
            warnings: Vec::new(),
            border: BorderStyle::default(),
            autolock: Duration::from_secs(300),
            last_input: Instant::now(),
            tick: 0,
            should_quit: false,
            node_url: None,
            solana_rpc: None,
            bitcoin_api: None,
            eth_rpc: None,
            polygon_rpc: None,
            base_rpc: None,
            ton_api: None,
            api_key: std::env::var("TRONGRID_API_KEY").ok(),
            bsc_api_key: std::env::var("NODEREAL_API_KEY").ok(),
            ton_api_key: std::env::var("TONCENTER_API_KEY").ok(),
            etherscan_key: std::env::var("ETHERSCAN_API_KEY").ok(),
            balances: None,
            balances_req: None,
            prices: neko_core::Prices::default(),
            balances_error: None,
            inflight: None,
            viewport: (MIN_COLS, MIN_ROWS),
            assets_pending: 0,
            assets_stale: false,
            // Follow the OS until the vault says otherwise. Constructing an
            // App deliberately does NOT touch the process-global locale:
            // `run` applies it once at startup, so building an App in a test
            // cannot reach out and change what another test is asserting on.
            locale: neko_i18n::Locale::detect(),
            startup_warning: None,
            req_seq: 0,
        }
    }

    pub fn next_req(&mut self) -> ReqId {
        self.req_seq += 1;
        ReqId(self.req_seq)
    }

    pub fn spinner(&self) -> &'static str {
        SPINNER[(self.tick as usize) % SPINNER.len()]
    }

    pub fn strength(&self) -> Strength {
        password::estimate(self.password.value())
    }

    /// Returns true if anything changed and a repaint is needed.
    pub fn on_tick(&mut self) -> bool {
        self.tick += 1;
        // Auto-lock. Order matters in Session::lock: the connection is dropped
        // first so SQLCipher wipes its own copy of the file key.
        if self.session.is_some() && self.last_input.elapsed() >= self.autolock {
            self.lock(LockReason::Idle);
            return true;
        }

        let mut dirty = false;

        // Recovery phrases hide themselves. Leaving words on screen because the
        // user got distracted is exactly the situation this guards against.
        let expired = matches!(
            &self.screen,
            Screen::Reveal { stage: RevealStage::Words { hide_at, .. }, .. }
                if Instant::now() >= *hide_at
        );
        if expired {
            self.pop();
            self.toast(neko_i18n::t(neko_i18n::Key::Reveal_Hidden));
            dirty = true;
        }

        if self
            .toast
            .as_ref()
            .is_some_and(|t| Instant::now() >= t.until)
        {
            self.toast = None;
            dirty = true;
        }

        // Anything showing a spinner has to repaint every tick, or it looks
        // frozen exactly when the user most wants to know something is running.
        let spinning = matches!(
            &self.screen,
            Screen::Send(st)
                if matches!(
                    st.step,
                    crate::send::SendStep::Quoting
                        | crate::send::SendStep::Broadcasting
                        | crate::send::SendStep::Authorize { checking: true, .. }
                )
        ) || matches!(
            &self.screen,
            Screen::Reveal {
                stage: RevealStage::Gate { checking: true, .. },
                ..
            }
        ) || matches!(&self.screen, Screen::History(h) if h.entries.is_none() && h.error.is_none())
            || self.assets_pending > 0;

        dirty
            || spinning
            || matches!(self.screen, Screen::Deriving { .. })
            || matches!(
                &self.screen,
                Screen::Reveal {
                    stage: RevealStage::Words { .. },
                    ..
                }
            )
    }

    pub fn lock(&mut self, reason: LockReason) {
        if let Some(mut s) = self.session.take() {
            s.lock();
        }
        self.password.clear();
        self.confirm.clear();
        self.screen = Screen::Locked { reason };
    }

    pub fn on_app_event(&mut self, ev: AppEvent) {
        match ev {
            AppEvent::Unlocked { req, res } => {
                // Drop replies we are no longer waiting for.
                let Screen::Deriving { req: want, .. } = self.screen else {
                    return;
                };
                if want != req {
                    return;
                }
                match res {
                    Ok(session) => {
                        self.password.clear();
                        self.confirm.clear();
                        self.error = None;
                        self.warnings.clear();
                        self.session = Some(session);
                        self.last_input = Instant::now();
                        self.load_settings();
                        // Wallets created before address bookkeeping existed
                        // have nothing to hang balances off; adopt them here.
                        if let Some(sess) = self.session.as_ref() {
                            let _ = sess.backfill_addresses();
                        }
                        self.nav.clear();
                        self.screen = self.wallets_screen();
                        self.assets_stale = true;
                    }
                    Err(e) => {
                        self.password.clear();
                        self.confirm.clear();
                        match e {
                            CoreError::WeakPassword(w) => {
                                self.warnings = w;
                                self.error = None;
                                self.screen = Screen::FirstRun {
                                    focus: SetupField::Password,
                                    warned: true,
                                };
                            }
                            other => {
                                self.error = Some(other.to_string());
                                self.screen = if self.db_path.exists() {
                                    Screen::Login {
                                        email_focused: false,
                                    }
                                } else {
                                    Screen::FirstRun {
                                        focus: SetupField::Email,
                                        warned: true,
                                    }
                                };
                            }
                        }
                    }
                }
            }

            AppEvent::Quoted { req, res } => self.on_quoted(req, res),
            AppEvent::Authorized { .. } | AppEvent::Blockhash { .. } => {
                // Handled in run.rs, which owns the event sender needed to
                // continue into the broadcast.
            }
            AppEvent::Broadcast { req, res } => self.on_broadcast(req, res),
            AppEvent::History { req, res } => {
                if self.inflight != Some(req) {
                    return;
                }
                self.inflight = None;
                if let Screen::History(h) = &mut self.screen {
                    match res {
                        Ok(entries) => {
                            h.entries = Some(entries);
                            h.error = None;
                        }
                        Err(e) => h.error = Some(e),
                    }
                }
            }
            AppEvent::WalletAssets {
                wallet_id,
                chain,
                res,
                ..
            } => self.on_wallet_assets(wallet_id, chain, res),
            AppEvent::Priced { chain, res, .. } => {
                if let Ok(p) = res {
                    self.prices.set_native(chain, p, now_secs());
                }
            }
            AppEvent::Balances { req, res } => {
                if self.balances_req != Some(req) {
                    return; // a reply for an address we already navigated away from
                }
                self.balances_req = None;
                match res {
                    Ok(b) => {
                        self.balances = Some(b);
                        self.balances_error = None;
                    }
                    Err(e) => {
                        self.balances = None;
                        self.balances_error = Some(e);
                    }
                }
            }
        }
    }

    /// Validate the setup form. Returns the reason it cannot proceed, if any.
    pub fn setup_blocker(&self) -> Option<&'static str> {
        if self.email.is_empty() || !self.email.value().contains('@') {
            return Some("email");
        }
        if !self.strength().acceptable() {
            return Some("weak");
        }
        if self.password.value() != self.confirm.value() {
            return Some("mismatch");
        }
        None
    }

    pub fn begin_unlock(&mut self, creating: bool) -> Option<UnlockJob> {
        if self.email.is_empty() || self.password.is_empty() {
            return None;
        }
        if creating && self.setup_blocker().is_some() {
            return None;
        }
        let req = self.next_req();
        self.screen = Screen::Deriving {
            req,
            started: Instant::now(),
            creating,
        };
        self.error = None;
        Some(UnlockJob {
            req,
            creating,
            path: self.db_path.clone(),
            email: self.email.value().to_string(),
            password: self.password.value().to_string(),
        })
    }
}

/// A unit of CPU-bound work for `spawn_blocking`.
pub struct UnlockJob {
    pub req: ReqId,
    pub creating: bool,
    pub path: PathBuf,
    pub email: String,
    pub password: String,
}

impl UnlockJob {
    /// Argon2id runs here. CPU-bound: never call this on the async runtime.
    pub fn run(self) -> AppEvent {
        let vf = VaultFile::at(&self.path);
        let res = if self.creating {
            // Calibrate on this machine rather than trusting a fixed default.
            // Measured spread is wide: BALANCED costs ~86 ms on an M-series
            // laptop but several hundred on older hardware, so a fixed profile
            // is either far too weak or uncomfortably slow depending on who
            // runs it. The chosen id goes in the plaintext header, so the vault
            // still opens anywhere -- just faster or slower.
            let profile = neko_vault::calibrate::recommend(None)
                .map(|c| c.profile)
                .unwrap_or(profile::DEFAULT);
            vf.create(&self.email, &self.password, profile)
        } else {
            vf.unlock(&self.email, &self.password)
        };
        AppEvent::Unlocked { req: self.req, res }
    }
}

impl std::fmt::Debug for UnlockJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockJob")
            .field("req", &self.req)
            .field("password", &"[redacted]")
            .finish()
    }
}

// ── Navigation and wallet actions ──────────────────────────────────────────

impl App {
    /// Build the wallet list from the session. Errors surface in-screen rather
    /// than tearing down the UI.
    pub fn wallets_screen(&mut self) -> Screen {
        match self.session.as_ref().map(|s| s.list_wallets()) {
            Some(Ok(items)) => Screen::Wallets(WalletsState::new(items)),
            Some(Err(e)) => {
                let mut st = WalletsState::new(Vec::new());
                st.error = Some(e.to_string());
                Screen::Wallets(st)
            }
            None => Screen::Locked {
                reason: LockReason::Manual,
            },
        }
    }

    pub fn refresh_wallets(&mut self) {
        let selected = match &self.screen {
            Screen::Wallets(w) => w.selected,
            _ => 0,
        };
        self.screen = self.wallets_screen();
        if let Screen::Wallets(w) = &mut self.screen {
            w.selected = selected.min(w.items.len().saturating_sub(1));
        }
    }

    pub fn push(&mut self, screen: Screen) {
        let current = std::mem::replace(&mut self.screen, screen);
        self.nav.push(current);
    }

    /// Esc. Returns false when there is nothing left to pop.
    pub fn pop(&mut self) -> bool {
        // Leaving the reveal screen must not leave words in memory.
        if let Screen::Reveal {
            stage: RevealStage::Words { words, .. },
            ..
        } = &mut self.screen
        {
            words.clear();
        }
        match self.nav.pop() {
            Some(prev) => {
                self.screen = prev;
                true
            }
            None => false,
        }
    }

    pub fn toast(&mut self, text: impl Into<String>) {
        self.toast = Some(Toast {
            text: text.into(),
            until: Instant::now() + Duration::from_secs(4),
        });
    }

    /// Copy, and describe the result honestly. OSC 52 is fire-and-forget: the
    /// terminal never replies, so we cannot claim the copy succeeded.
    pub fn copy_to_clipboard(&mut self, text: &str) {
        match self.clipboard.copy(text) {
            Some(crate::clipboard::CopyOutcome::Confirmed) => {
                self.toast(neko_i18n::t(neko_i18n::Key::Common_Copied))
            }
            Some(crate::clipboard::CopyOutcome::Sent) => {
                self.toast(neko_i18n::t(neko_i18n::Key::Common_CopySent))
            }
            None => self.toast(neko_i18n::t(neko_i18n::Key::Common_ClipboardUnavailable)),
        }
    }

    pub fn open_chains(&mut self) {
        let Screen::Wallets(w) = &self.screen else {
            return;
        };
        let Some(cur) = w.current() else { return };
        let (wallet_id, name) = (cur.id, cur.label.clone());
        self.push(Screen::Chains {
            wallet_id,
            name,
            selected: 0,
        });
    }

    pub fn open_assets(&mut self, tx: &crate::keys::Sender) {
        let Screen::Chains {
            wallet_id,
            name,
            selected,
        } = &self.screen
        else {
            return;
        };
        let chain = crate::nav::CHAINS[*selected];
        let (wallet_id, name) = (*wallet_id, name.clone());
        let address = match self
            .session
            .as_ref()
            .map(|s| s.address_of(wallet_id, chain, 0))
        {
            Some(Ok(a)) => a.to_string(),
            Some(Err(e)) => {
                self.toast(e.to_string());
                return;
            }
            None => return,
        };
        self.push(Screen::Assets {
            wallet_id,
            name,
            chain,
            address,
            selected: 0,
        });
        self.fetch_balances(tx);
    }

    /// Ask the chain for this address's balances.
    ///
    /// Fire-and-forget onto the runtime; the reply comes back as an AppEvent
    /// matched by ReqId, so a late response for a wallet the user already
    /// navigated away from is dropped rather than painted over the new one.
    pub fn fetch_balances(&mut self, tx: &crate::keys::Sender) {
        let Screen::Assets { address, chain, .. } = &self.screen else {
            return;
        };
        let chain = *chain;
        let Ok(addr) = neko_core::ChainAddress::parse(chain, address) else {
            return;
        };
        let id = self.next_req();
        self.balances_req = Some(id);
        self.balances = None;
        self.balances_error = None;
        let client = self.chain_client(chain);
        let tx = tx.clone();
        tokio::spawn(async move {
            let res = crate::chain::wallet_assets(&client, addr).await;
            let _ = tx.send(crate::event::AppEvent::Balances { req: id, res });
        });
    }

    /// Enter the reveal flow. Always starts at the gate, never at the words.
    pub fn open_reveal(&mut self) {
        let (wallet_id, name) = match &self.screen {
            Screen::Wallets(w) => match w.current() {
                Some(c) => (c.id, c.label.clone()),
                None => return,
            },
            Screen::Chains {
                wallet_id, name, ..
            } => (*wallet_id, name.clone()),
            _ => return,
        };
        self.push(Screen::Reveal {
            wallet_id,
            name,
            stage: RevealStage::Gate {
                password: Field::new(true),
                checking: false,
            },
        });
    }

    pub fn wallet_form(&mut self, form: WalletForm) {
        if let Screen::Wallets(w) = &mut self.screen {
            w.error = None;
            w.form = Some(form);
        }
    }

    pub fn close_form(&mut self) {
        if let Screen::Wallets(w) = &mut self.screen {
            w.form = None;
        }
    }
}

// ── Transfers ──────────────────────────────────────────────────────────────

impl App {
    /// A client for one chain. The configured node URL applies to TRON only -
    /// pointing a BNB Chain RPC at a TronGrid endpoint would fail in a way
    /// that looks like the network being down.
    pub fn chain_client(&self, chain: neko_core::ChainId) -> crate::chain::Client {
        let url = match chain {
            neko_core::ChainId::Tron => self.node_url.as_deref(),
            neko_core::ChainId::Solana => self.solana_rpc.as_deref(),
            neko_core::ChainId::Bitcoin => self.bitcoin_api.as_deref(),
            neko_core::ChainId::Ethereum => self.eth_rpc.as_deref(),
            neko_core::ChainId::Polygon => self.polygon_rpc.as_deref(),
            neko_core::ChainId::Base => self.base_rpc.as_deref(),
            neko_core::ChainId::Ton => self.ton_api.as_deref(),
            neko_core::ChainId::Bsc => None,
        };
        let key = match chain {
            neko_core::ChainId::Tron => self.api_key.clone(),
            // The same NodeReal key works on both EVM chains; only the host
            // differs, and that comes from the chain.
            neko_core::ChainId::Bsc | neko_core::ChainId::Ethereum => self.bsc_api_key.clone(),
            // NodeReal serves neither of these, so there is no key to pass;
            // they read from Blockscout, or from Etherscan when a key is set.
            neko_core::ChainId::Polygon | neko_core::ChainId::Base => None,
            // Neither Solana's public cluster nor Esplora needs a key. Both
            // rate-limit, which costs a retry rather than a screen.
            neko_core::ChainId::Solana | neko_core::ChainId::Bitcoin => None,
            // toncenter's public limit is low enough that a balance refresh
            // can hit it; a key raises it. Optional, like the others.
            neko_core::ChainId::Ton => self.ton_api_key.clone(),
        };
        crate::chain::Client::for_chain_with(chain, url, key, self.etherscan_key.clone())
    }

    /// Open the transfer flow for the asset highlighted on the assets screen.
    pub fn open_send(&mut self, tx: &crate::keys::Sender) {
        let Screen::Assets {
            wallet_id,
            name,
            address,
            chain,
            selected,
            ..
        } = &self.screen
        else {
            return;
        };
        let chain = *chain;
        let from = match neko_core::ChainAddress::parse(chain, address) {
            Ok(a) => a,
            Err(_) => return,
        };
        // Driven by what the chain actually carries, rather than by assuming
        // two. Bitcoin has one asset, and a fixed pair would either hide it or
        // invent a second.
        let assets = chain.assets();
        let Some(asset) = assets.get(*selected).copied() else {
            return;
        };
        let label = asset.symbol().to_string();
        let mut state = crate::send::SendState::new(*wallet_id, name.clone(), from, asset, label);
        // Matched by symbol rather than by the row's position, so a change to
        // the order of the assets screen cannot quietly offer the wrong
        // balance as the maximum.
        state.balance = self.balances.as_ref().and_then(|rows| {
            rows.iter()
                .find(|(sym, _, _)| sym == asset.symbol())
                .map(|(_, _, amt)| *amt)
        });
        // Carry the counterparties we have seen so the destination can be
        // checked against them for a crafted lookalike.
        state.known = self.known_counterparties();
        self.push(Screen::Send(Box::new(state)));
        // The review screen prices the fee, and the wallet list is the only
        // other place that fetches. Reaching send without passing through it -
        // or with that fetch having failed - would leave the fee as a bare gas
        // figure, which is the thing this is here to avoid.
        if self.prices.is_empty() {
            self.fetch_prices(tx);
        }
    }

    fn on_quoted(&mut self, req: ReqId, res: Result<Box<crate::event::Quote>, String>) {
        if self.inflight != Some(req) {
            return; // a reply the user already navigated away from
        }
        self.inflight = None;
        let Screen::Send(s) = &mut self.screen else {
            return;
        };
        match res {
            Ok(q) => {
                let params = q.tx_params();
                let mut fee = match *q {
                    crate::event::Quote::Tron {
                        energy,
                        bandwidth_needed,
                        resources,
                        prices,
                        recipient_is_new,
                        ..
                    } => crate::send::FeeQuote::Tron(crate::send::TronFee {
                        energy_base: energy.base(),
                        energy_penalty: energy.penalty,
                        bandwidth_needed,
                        available: resources.map(|r| {
                            (
                                (r.energy_available, r.energy_limit),
                                (r.bandwidth_available, r.bandwidth_limit),
                            )
                        }),
                        prices: prices.map(|p| (p.sun_per_energy, p.sun_per_bandwidth)),
                        recipient_is_new,
                    }),
                    crate::event::Quote::Evm {
                        chain,
                        params: p,
                        native_balance,
                        sending_native,
                        amount,
                    } => crate::send::FeeQuote::Evm(crate::send::EvmFee {
                        chain,
                        gas_limit: p.gas_limit,
                        fees: p.fees,
                        native_balance,
                        sending_native,
                        amount,
                    }),
                    crate::event::Quote::Solana {
                        params: p,
                        sol_balance,
                        sending_native,
                        amount,
                        rent,
                    } => crate::send::FeeQuote::Solana(crate::send::SolanaFee {
                        compute_units: p.compute_unit_limit,
                        compute_unit_price: p.compute_unit_price,
                        rent,
                        sol_balance,
                        sending_native,
                        amount,
                    }),
                    crate::event::Quote::Ton {
                        params: ref p,
                        gram_balance,
                        sending_native,
                        amount,
                        fee,
                        attached,
                    } => crate::send::FeeQuote::Ton(crate::send::TonFee {
                        fee,
                        attached,
                        gram_balance,
                        sending_native,
                        amount,
                        deploy: p.deploy,
                    }),
                    crate::event::Quote::Bitcoin {
                        fee_rate,
                        balance,
                        utxo_count,
                        ref selection,
                        ..
                    } => crate::send::FeeQuote::Bitcoin(crate::send::BtcFee {
                        fee_rate,
                        vbytes: selection.vbytes,
                        fee: selection.fee,
                        inputs: selection.inputs.len(),
                        utxo_count,
                        balance,
                        change: selection.change,
                        change_was_dust: selection.change_was_dust,
                    }),
                };

                // "Send everything" can only be finished here. On a native
                // transfer the fee comes out of the very balance being sent, so
                // the maximum does not exist until the fee does - which is why
                // the amount screen could only record the request.
                if let Some(bal) = fee.native_balance().or(s.balance) {
                    // The reserve, not the expected cost. On EIP-1559 the chain
                    // requires the balance to cover the whole ceiling even
                    // though the difference comes straight back, so holding
                    // back the expected figure produces an amount the node
                    // rejects with "insufficient funds".
                    let held = fee.reserve();
                    if let Some(max) = s.hold_back_fee(bal, held) {
                        // The quote decides affordability from the amount it was
                        // given, so it has to be told the amount changed.
                        fee.set_amount(max);
                    }
                }

                // Built after the adjustment, so the request, the review screen
                // and what gets signed all read the one amount.
                let request = match s.build_request() {
                    Ok(r) => r,
                    Err(e) => {
                        s.step = crate::send::SendStep::Failed(e.to_string());
                        return;
                    }
                };
                s.step = crate::send::SendStep::Review {
                    req: Box::new(request),
                    params: Box::new(params),
                    quote: Some(Box::new(fee)),
                    typed: Field::new(false),
                };
            }
            Err(e) => s.step = crate::send::SendStep::Failed(e),
        }
    }

    fn on_broadcast(&mut self, req: ReqId, res: Result<String, String>) {
        if self.inflight != Some(req) {
            return;
        }
        self.inflight = None;
        let Screen::Send(s) = &mut self.screen else {
            return;
        };
        // The explorer belongs to the chain that was sent on. This used to be
        // TRON's for all six, so a Solana signature was offered as a tronscan
        // link that could only ever say "not found".
        let chain = s.asset.chain();
        s.step = match res {
            Ok(txid) => crate::send::SendStep::Done {
                explorer: chain.explorer_tx(&txid),
                txid,
            },
            Err(e) => crate::send::SendStep::Failed(e),
        };
    }
}

// ── Settings ───────────────────────────────────────────────────────────────

impl App {
    /// Pull the persisted chain configuration out of the freshly unlocked vault.
    pub fn load_settings(&mut self) {
        use neko_store::repo::settings::keys;
        let Some(s) = self.session.as_ref() else {
            return;
        };

        if let Ok(v) = s.setting(keys::NODE_URL) {
            self.node_url = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::SOLANA_RPC) {
            self.solana_rpc = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::BITCOIN_API) {
            self.bitcoin_api = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::ETH_RPC) {
            self.eth_rpc = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::POLYGON_RPC) {
            self.polygon_rpc = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::BASE_RPC) {
            self.base_rpc = v.filter(|u| !u.is_empty());
        }
        if let Ok(v) = s.setting(keys::TON_API) {
            self.ton_api = v.filter(|u| !u.is_empty());
        }
        // An env var still wins, so a throwaway key can be supplied per run.
        if self.api_key.is_none() {
            if let Ok(Some(k)) = s.secret_setting(keys::API_KEY) {
                self.api_key = Some(k.to_string());
            }
        }
        if self.bsc_api_key.is_none() {
            if let Ok(Some(k)) = s.secret_setting(keys::BSC_API_KEY) {
                self.bsc_api_key = Some(k.to_string());
            }
        }
        if self.etherscan_key.is_none() {
            if let Ok(Some(k)) = s.secret_setting(keys::ETHERSCAN_KEY) {
                self.etherscan_key = Some(k.to_string());
            }
        }
        if self.ton_api_key.is_none() {
            if let Ok(Some(k)) = s.secret_setting(keys::TON_API_KEY) {
                self.ton_api_key = Some(k.to_string());
            }
        }
        // A stored choice outranks OS detection: the user said what they
        // want, and it must survive every restart.
        if let Ok(Some(v)) = s.setting(keys::LANGUAGE) {
            if let Some(l) = neko_i18n::Locale::from_tag(&v) {
                self.locale = l;
                neko_i18n::set_locale(l);
            }
        }
        if let Ok(Some(v)) = s.setting(keys::AUTOLOCK_SECS) {
            if let Ok(secs) = v.parse::<u64>() {
                if (60..=3600).contains(&secs) {
                    self.autolock = Duration::from_secs(secs);
                }
            }
        }
    }

    pub fn set_bsc_api_key(&mut self, key: &str) {
        if let Some(s) = self.session.as_ref() {
            let _ = s.set_secret_setting(neko_store::repo::settings::keys::BSC_API_KEY, key);
        }
        self.bsc_api_key = Some(key.to_string()).filter(|k| !k.is_empty());
    }

    pub fn set_etherscan_key(&mut self, key: &str) {
        if let Some(s) = self.session.as_ref() {
            let _ = s.set_secret_setting(neko_store::repo::settings::keys::ETHERSCAN_KEY, key);
        }
        self.etherscan_key = Some(key.to_string()).filter(|k| !k.is_empty());
    }

    pub fn set_ton_api_key(&mut self, key: &str) {
        if let Some(s) = self.session.as_ref() {
            let _ = s.set_secret_setting(neko_store::repo::settings::keys::TON_API_KEY, key);
        }
        self.ton_api_key = Some(key.to_string()).filter(|k| !k.is_empty());
    }

    pub fn set_api_key(&mut self, key: &str) {
        if let Some(s) = self.session.as_ref() {
            let _ = s.set_secret_setting(neko_store::repo::settings::keys::API_KEY, key);
        }
        self.api_key = Some(key.to_string()).filter(|k| !k.is_empty());
    }
}

impl App {
    pub fn open_settings(&mut self) {
        self.push(Screen::Settings(SettingsState::new()));
    }

    /// Human-readable current value for each settings row.
    pub fn setting_value(&self, row: crate::nav::SettingRow) -> String {
        use crate::nav::SettingRow;
        match row {
            // Shown in its own script so somebody who cannot read the current
            // language can still find their way back.
            SettingRow::Language => self.locale.endonym().to_string(),
            SettingRow::BscApiKey => match &self.bsc_api_key {
                Some(k) if k.len() > 4 => neko_i18n::tf(
                    neko_i18n::Key::Settings_ApiKeySet,
                    &[("tail", &k[k.len() - 4..])],
                ),
                Some(_) => neko_i18n::tf(neko_i18n::Key::Settings_ApiKeySet, &[("tail", "")]),
                None => neko_i18n::t(neko_i18n::Key::Settings_BscApiKeyUnset).to_string(),
            },
            SettingRow::EtherscanKey => match &self.etherscan_key {
                Some(k) if k.len() > 4 => neko_i18n::tf(
                    neko_i18n::Key::Settings_ApiKeySet,
                    &[("tail", &k[k.len() - 4..])],
                ),
                Some(_) => neko_i18n::tf(neko_i18n::Key::Settings_ApiKeySet, &[("tail", "")]),
                None => neko_i18n::t(neko_i18n::Key::Settings_EtherscanKeyUnset).to_string(),
            },
            SettingRow::TonApiKey => match &self.ton_api_key {
                Some(k) if k.len() > 4 => neko_i18n::tf(
                    neko_i18n::Key::Settings_ApiKeySet,
                    &[("tail", &k[k.len() - 4..])],
                ),
                Some(_) => neko_i18n::tf(neko_i18n::Key::Settings_ApiKeySet, &[("tail", "")]),
                None => neko_i18n::t(neko_i18n::Key::Settings_TonApiKeyUnset).to_string(),
            },
            SettingRow::ApiKey => match &self.api_key {
                // Never render a credential, even one the user typed.
                Some(k) if k.len() > 4 => neko_i18n::tf(
                    neko_i18n::Key::Settings_ApiKeySet,
                    &[("tail", &k[k.len() - 4..])],
                ),
                Some(_) => neko_i18n::tf(neko_i18n::Key::Settings_ApiKeySet, &[("tail", "")]),
                None => neko_i18n::t(neko_i18n::Key::Settings_ApiKeyUnset).to_string(),
            },
            SettingRow::NodeUrl => self
                .node_url
                .clone()
                .unwrap_or_else(|| neko_tron::DEFAULT_URL.into()),
            SettingRow::SolanaRpc => self
                .solana_rpc
                .clone()
                .unwrap_or_else(|| neko_solana::DEFAULT_RPC.into()),
            SettingRow::BitcoinApi => self
                .bitcoin_api
                .clone()
                .unwrap_or_else(|| neko_btc::DEFAULT_API.into()),
            SettingRow::EthRpc => self
                .eth_rpc
                .clone()
                .unwrap_or_else(|| neko_evm::ETHEREUM.default_rpc.into()),
            SettingRow::PolygonRpc => self
                .polygon_rpc
                .clone()
                .unwrap_or_else(|| neko_evm::POLYGON.default_rpc.into()),
            SettingRow::BaseRpc => self
                .base_rpc
                .clone()
                .unwrap_or_else(|| neko_evm::BASE.default_rpc.into()),
            SettingRow::TonApi => self
                .ton_api
                .clone()
                .unwrap_or_else(|| neko_ton::DEFAULT_API.into()),
            SettingRow::AutoLock => neko_i18n::tf(
                neko_i18n::Key::Settings_Minutes,
                &[("n", &(self.autolock.as_secs() / 60).to_string())],
            ),
            SettingRow::BorderStyle => match self.border {
                BorderStyle::Unicode => {
                    neko_i18n::t(neko_i18n::Key::Settings_BorderUnicode).to_string()
                }
                BorderStyle::Ascii => {
                    neko_i18n::t(neko_i18n::Key::Settings_BorderAscii).to_string()
                }
            },
        }
    }
}

impl App {
    /// A password check that can run on a blocking thread.
    ///
    /// Returns a closure rather than borrowing the session, because the session
    /// is not `Send` and must stay on this thread. The closure opens the vault
    /// file independently, which is exactly the check we want: it proves the
    /// password, not that a session happens to exist.
    pub fn password_verifier(&self) -> impl Fn(&std::path::Path, &str) -> bool + Send + 'static {
        let email = self.session.as_ref().map(|s| s.email().to_string());
        move |path: &std::path::Path, password: &str| match &email {
            Some(e) => neko_core::VaultFile::at(path).unlock(e, password).is_ok(),
            None => false,
        }
    }
}

impl App {
    pub fn open_history(&mut self, tx: &crate::keys::Sender) {
        let Screen::Assets { address, chain, .. } = &self.screen else {
            return;
        };
        let (chain, address) = (*chain, address.clone());
        let mut state = crate::nav::HistoryState::new(chain, address);
        state.page = crate::nav::history_page_for(self.viewport.1);
        self.push(Screen::History(state));
        self.fetch_history(tx);
    }

    pub fn fetch_history(&mut self, tx: &crate::keys::Sender) {
        let Screen::History(h) = &mut self.screen else {
            return;
        };
        let chain = h.chain;
        let Ok(addr) = neko_core::ChainAddress::parse(chain, &h.address) else {
            return;
        };
        h.error = None;
        h.entries = None;
        let id = self.next_req();
        self.inflight = Some(id);
        let client = self.chain_client(chain);
        let tx = tx.clone();
        tokio::spawn(async move {
            let res = crate::chain::history(&client, addr, 50).await;
            let _ = tx.send(crate::event::AppEvent::History { req: id, res });
        });
    }
}

impl App {
    /// Every counterparty seen in a history screen this session, dust included:
    /// a poisoning address is exactly the one we want to compare against.
    pub fn known_counterparties(&self) -> Vec<String> {
        let mut out = Vec::new();
        let screens = self.nav.iter().chain(std::iter::once(&self.screen));
        for s in screens {
            if let Screen::History(h) = s {
                if let Some(entries) = &h.entries {
                    for e in entries {
                        if !out.contains(&e.counterparty) {
                            out.push(e.counterparty.clone());
                        }
                    }
                }
            }
        }
        out
    }
}

impl App {
    /// Record the terminal size and resize any list that pages by screenful.
    ///
    /// Called before every draw, so a window resize takes effect immediately
    /// rather than leaving PageDown jumping by the wrong amount.
    pub fn set_viewport(&mut self, width: u16, height: u16) {
        if self.viewport == (width, height) {
            return;
        }
        self.viewport = (width, height);
        let page = crate::nav::history_page_for(height);
        if let Screen::History(h) = &mut self.screen {
            h.page = page;
            // Keep the cursor inside the new window.
            if h.selected >= h.offset + page {
                h.offset = h.selected + 1 - page;
            }
        }
        for s in self.nav.iter_mut() {
            if let Screen::History(h) = s {
                h.page = page;
            }
        }
    }
}

// ── Wallet asset refresh ───────────────────────────────────────────────────

impl App {
    /// Kick off a background balance fetch for every wallet in the list.
    ///
    /// The list itself already rendered from cache, so this only ever replaces
    /// numbers that are visibly labelled as stale. Requests are per-wallet so a
    /// single slow or failing address does not hold up the rest.
    pub fn refresh_wallet_assets(&mut self, tx: &crate::keys::Sender) {
        // One request per wallet *per chain*: a wallet is an account on each,
        // and a slow or failing one must not hold up the others.
        let targets: Vec<(i64, neko_core::ChainId, String)> = match &self.screen {
            Screen::Wallets(w) => w
                .items
                .iter()
                .flat_map(|i| {
                    i.addresses
                        .iter()
                        .map(|(c, a)| (i.id, *c, a.clone()))
                        .collect::<Vec<_>>()
                })
                .collect(),
            _ => return,
        };
        if targets.is_empty() {
            return;
        }
        self.fetch_prices(tx);
        self.assets_pending = targets.len();
        for (wallet_id, chain, address) in targets {
            let Ok(addr) = neko_core::ChainAddress::parse(chain, &address) else {
                self.assets_pending = self.assets_pending.saturating_sub(1);
                continue;
            };
            let id = self.next_req();
            let client = self.chain_client(chain);
            let tx = tx.clone();
            tokio::spawn(async move {
                let res = crate::chain::wallet_assets(&client, addr).await;
                let _ = tx.send(crate::event::AppEvent::WalletAssets {
                    req: id,
                    wallet_id,
                    chain,
                    res,
                });
            });
        }
    }

    /// One quote per chain, not per wallet: the price is a property of the
    /// chain, and asking once is both faster and kinder to the node.
    pub fn fetch_prices(&mut self, tx: &crate::keys::Sender) {
        for chain in neko_core::CHAINS {
            let id = self.next_req();
            let client = self.chain_client(chain);
            let tx = tx.clone();
            tokio::spawn(async move {
                let res = crate::chain::native_price(&client).await;
                let _ = tx.send(crate::event::AppEvent::Priced {
                    req: id,
                    chain,
                    res,
                });
            });
        }
    }

    fn on_wallet_assets(
        &mut self,
        wallet_id: i64,
        chain: neko_core::ChainId,
        res: Result<Vec<(String, u8, i128)>, String>,
    ) {
        self.assets_pending = self.assets_pending.saturating_sub(1);
        let Ok(assets) = res else { return };

        // Persist first, then re-read, so what is displayed is exactly what a
        // restart would show.
        if let Some(s) = self.session.as_ref() {
            let _ = s.cache_assets(wallet_id, chain, &assets);
            if let Ok(fresh) = s.cached_assets(wallet_id, chain) {
                if let Screen::Wallets(w) = &mut self.screen {
                    if let Some(item) = w.items.iter_mut().find(|i| i.id == wallet_id) {
                        match item.assets.iter_mut().find(|(c, _)| *c == chain) {
                            Some(slot) => slot.1 = fresh,
                            None => item.assets.push((chain, fresh)),
                        }
                    }
                }
            }
        }
    }
}

impl App {
    /// Move to the next language and remember it.
    ///
    /// Switching is a single atomic store: nothing is reloaded, so the very
    /// next frame is already in the new language.
    pub fn cycle_language(&mut self, forward: bool) {
        let all = neko_i18n::LOCALES;
        let i = all.iter().position(|l| *l == self.locale).unwrap_or(0);
        let next = if forward {
            (i + 1) % all.len()
        } else {
            (i + all.len() - 1) % all.len()
        };
        self.set_language(all[next]);
    }

    /// Push this app's language into the global used by `t()`. Called once at
    /// startup and whenever the language changes.
    pub fn apply_locale(&self) {
        neko_i18n::set_locale(self.locale);
    }

    pub fn set_language(&mut self, l: neko_i18n::Locale) {
        self.locale = l;
        neko_i18n::set_locale(l);
        if let Some(s) = self.session.as_ref() {
            let _ = s.set_setting(neko_store::repo::settings::keys::LANGUAGE, l.tag());
        }
    }
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}
