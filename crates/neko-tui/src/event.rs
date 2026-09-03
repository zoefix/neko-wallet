//! Messages crossing the boundary between the render loop and background work.

use neko_core::{CoreError, Session};

/// Identifies one in-flight async request.
///
/// This is not decoration. Without it you get the classic bug: the user starts
/// an unlock, hits Esc, starts another, and the first one's late reply lands on
/// top of the second. Each screen remembers the `ReqId` it is waiting for and
/// silently drops anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReqId(pub u64);

/// What the chain contributed, before anything is signed.
pub struct Quote {
    pub params: neko_tron::tx::TxParams,
    /// Energy this transfer needs, simulated against the chain and split into
    /// its base cost and the dynamic-energy surcharge.
    pub energy: neko_tron::EnergyEstimate,
    /// Bytes on the wire; bandwidth is charged per byte.
    pub bandwidth_needed: i64,
    /// What the account can cover without burning anything.
    ///
    /// `None` means the lookup failed, which is NOT the same as zero. Treating
    /// a rate-limited request as "you have nothing" quietly overstates the fee
    /// and tells the user something false about their own account.
    pub resources: Option<neko_tron::Resources>,
    /// `None` means the chain parameters could not be read.
    pub prices: Option<neko_tron::Prices>,
    /// A first-time recipient of this token pays to create a storage slot,
    /// which costs considerably more energy.
    pub recipient_is_new: bool,
}

pub enum AppEvent {
    /// Argon2id finished (or failed) in the blocking pool.
    Unlocked {
        req: ReqId,
        res: Result<Session, CoreError>,
    },
    /// Block reference plus an energy estimate, ready to build the transaction.
    Quoted {
        req: ReqId,
        res: Result<Box<Quote>, String>,
    },
    /// The password typed on the final send gate was (or was not) correct.
    Authorized { req: ReqId, ok: bool },
    /// The network accepted (or rejected) a signed transaction.
    Broadcast {
        req: ReqId,
        res: Result<String, String>,
    },
    /// Transaction history for the address currently on screen.
    History {
        req: ReqId,
        res: Result<Vec<neko_tron::HistoryEntry>, String>,
    },
    /// Balances for one wallet in the list, fetched in the background so the
    /// list itself renders from cache without waiting.
    WalletAssets {
        req: ReqId,
        wallet_id: i64,
        res: Result<Vec<(String, u8, i128)>, String>,
    },
    /// Balances for the address currently on screen.
    Balances {
        req: ReqId,
        res: Result<Vec<(String, String)>, String>,
    },
}

/// Hand-written so a Session or a transaction can never be printed by accident.
impl std::fmt::Debug for AppEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (name, req, ok) = match self {
            AppEvent::Unlocked { req, res } => ("Unlocked", req, res.is_ok()),
            AppEvent::Quoted { req, res } => ("Quoted", req, res.is_ok()),
            AppEvent::Authorized { req, ok } => ("Authorized", req, *ok),
            AppEvent::Broadcast { req, res } => ("Broadcast", req, res.is_ok()),
            AppEvent::History { req, res } => ("History", req, res.is_ok()),
            AppEvent::WalletAssets { req, res, .. } => ("WalletAssets", req, res.is_ok()),
            AppEvent::Balances { req, res } => ("Balances", req, res.is_ok()),
        };
        f.debug_struct(name)
            .field("req", req)
            .field("ok", &ok)
            .finish()
    }
}
