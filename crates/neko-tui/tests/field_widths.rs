//! Labelled input fields have to hold what goes in them.
//!
//! Both halves of one of these lines were fixed numbers: ten cells for the
//! label and forty for the value. Neither was enough. Forty cut every address
//! longer than it - an EVM address is 42 characters, a Solana one up to 44, a
//! Bitcoin P2WSH or Taproot address 62 - so the one thing a person needs to
//! check before sending money was the thing being hidden. Ten cut
//! `メールアドレス` on the login screen and `私钥 (十六进制)` on the import form.

use neko_tui::app::{App, Screen};
use neko_tui::nav::{SettingsState, WalletForm};
use neko_tui::send::{SendState, SendStep};

/// The longest address each chain can produce.
fn longest_address(chain: neko_core::ChainId) -> &'static str {
    match chain {
        neko_core::ChainId::Tron => "TUEZSdKsoDHQMeZwihtdoBiN46zxhGWYdH",
        neko_core::ChainId::Bsc
        | neko_core::ChainId::Ethereum
        | neko_core::ChainId::Polygon
        | neko_core::ChainId::Base
        | neko_core::ChainId::Arbitrum
        | neko_core::ChainId::Optimism => "0xA41811CF4D41e306310CB82B47258C22b80475cC",
        neko_core::ChainId::Solana => "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM",
        // P2WSH: 62 characters, the longest string this wallet ever has to
        // show in a field.
        neko_core::ChainId::Bitcoin => {
            "bc1qrp33g0q5c5txsp9arysrx4k6zdkfs4nce4xj0gdcccefvpysxf3qccfmv3"
        }
        // Always 48: a workchain byte, 32 of hash and a CRC, base64url'd with
        // nothing left over to pad.
        neko_core::ChainId::Ton => "EQAzWZa6nM5mJev91wGc7VCSfBoIsYRqKJpV78N8Add9-U9d",
    }
}

fn render(app: &App, cols: u16) -> String {
    let mut term = ratatui::Terminal::new(ratatui::backend::TestBackend::new(cols, 26)).unwrap();
    term.draw(|f| neko_tui::render::draw(f, app)).unwrap();
    let buf = term.backend().buffer().clone();
    (0..buf.area.height)
        .map(|y| {
            let mut line = String::new();
            let mut x = 0u16;
            while x < buf.area.width {
                let sym = buf[(x, y)].symbol();
                line.push_str(sym);
                x += unicode_width::UnicodeWidthStr::width(sym).max(1) as u16;
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A short address for the sender, so that finding the long one on screen can
/// only mean the destination field is showing it.
fn own_address(chain: neko_core::ChainId) -> &'static str {
    match chain {
        neko_core::ChainId::Bitcoin => "bc1qcr8te4kr609gcawutmrza0j4xv80jy8z306fyu",
        _ => longest_address(chain),
    }
}

fn sending(chain: neko_core::ChainId, cols: u16) -> App {
    let mut st = SendState::new(
        1,
        "w".into(),
        neko_core::ChainAddress::parse(chain, own_address(chain)).unwrap(),
        chain.native(),
        chain.native_symbol().to_string(),
    );
    longest_address(chain).chars().for_each(|c| st.to.push(c));
    st.step = SendStep::EnterAmount;
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fields.db"));
    app.screen = Screen::Send(Box::new(st));
    app.set_viewport(cols, 26);
    app
}

/// The destination a transfer is about to go to must be readable in full.
///
/// At a terminal wide enough to hold it - which every one of these is, at 100
/// columns - nothing may be elided.
#[test]
fn a_destination_is_shown_in_full_when_it_fits() {
    for chain in neko_core::CHAINS {
        for locale in neko_i18n::LOCALES {
            neko_i18n::set_locale(locale);
            let app = sending(chain, 100);
            let out = render(&app, 100);
            assert!(
                out.contains(longest_address(chain)),
                "{chain:?} in {locale:?}: the destination is cut off\n{out}"
            );
        }
    }
    neko_i18n::set_locale(neko_i18n::Locale::English);
}

/// The field grows with the terminal rather than sitting at a fixed forty.
#[test]
fn the_field_uses_the_width_it_is_given() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let chain = neko_core::ChainId::Bitcoin;
    let addr = longest_address(chain);

    // Too narrow to hold it: elided, and the *tail* survives - that is the part
    // the confirmation step asks to be retyped, and the part under the cursor
    // while typing.
    let narrow = render(&sending(chain, 80), 80);
    assert!(
        !narrow.contains(addr),
        "80 columns cannot hold 62 characters"
    );
    assert!(
        narrow.contains(&addr[addr.len() - 30..]),
        "the end of the address was dropped instead of the start:\n{narrow}"
    );

    // Wide enough: shown whole.
    assert!(render(&sending(chain, 100), 100).contains(addr));
}

/// A label that reads as cut off makes the whole form look broken, and three
/// of these were.
#[test]
fn no_label_is_truncated_in_any_language() {
    for locale in neko_i18n::LOCALES {
        neko_i18n::set_locale(locale);

        let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fields.db"));
        app.set_viewport(100, 26);

        // The login screen, the import form, and the send screen: the three
        // that carry the longest labels.
        app.screen = Screen::Login {
            email_focused: true,
        };
        let login = render(&app, 100);

        app.screen = Screen::Wallets(neko_tui::nav::WalletsState::new(Vec::new()));
        if let Screen::Wallets(w) = &mut app.screen {
            w.form = Some(WalletForm::ImportPrivkey {
                label: neko_tui::input::Field::new(false),
                hex: neko_tui::input::Field::new(true),
                focus: 0,
            });
        }
        let import = render(&app, 100);

        let send = render(&sending(neko_core::ChainId::Bsc, 100), 100);

        for (what, out) in [("login", login), ("import", import), ("send", send)] {
            for line in out.lines() {
                // A label is elided only ever at the label column, and the
                // marker is the only thing that says so.
                assert!(
                    !line.contains('~') || line.contains('['),
                    "{locale:?} {what}: a label was truncated\n{line}"
                );
            }
        }
    }
    neko_i18n::set_locale(neko_i18n::Locale::English);
}

/// Settings values are shown, not typed into, and are their own column - but
/// a node URL is long and must not be silently shortened either.
#[test]
fn a_configured_node_url_is_readable() {
    neko_i18n::set_locale(neko_i18n::Locale::English);
    let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fields.db"));
    app.eth_rpc = Some("https://mainnet.example.com/v2/some-fairly-long-key".into());
    app.screen = Screen::Settings(SettingsState::new());
    app.set_viewport(110, 26);
    let out = render(&app, 110);
    assert!(
        out.contains("https://mainnet.example.com/v2/some-fairly-long-key"),
        "the configured endpoint is not readable:\n{out}"
    );
}

/// The password step shows two facts and asks for one thing, and all three have
/// to start in the same column.
///
/// They did not. The amount's label was padded with two trailing spaces, the
/// destination's with a different constant, and the password is a field with a
/// focus marker and a label column of its own - so one screen had its three
/// values at three different places, and in Japanese the labels are wider,
/// which moved two of them again.
#[test]
fn the_password_step_lines_up() {
    use neko_tui::input::Field;

    const TO: &str = "0x74224e8D997f1C438cbFd2cE147C8bbDCd5FA0c8";

    for locale in neko_i18n::LOCALES {
        neko_i18n::set_locale(locale);

        let chain = neko_core::ChainId::Ethereum;
        let mut st = SendState::new(
            1,
            "w".into(),
            neko_core::ChainAddress::parse(chain, own_address(chain)).unwrap(),
            chain.native(),
            "ETH".into(),
        );
        TO.chars().for_each(|c| st.to.push(c));
        "0.5".chars().for_each(|c| st.amount.push(c));
        let req = st.build_request().unwrap();
        st.step = SendStep::Authorize {
            req: Box::new(req),
            params: Box::new(neko_core::ChainTxParams::Evm(neko_evm::tx::TxParams {
                nonce: 0,
                gas_limit: 21_000,
                chain_id: 1,
                fees: neko_evm::tx::Fees::Legacy { gas_price: 1 },
            })),
            password: Field::new(true),
            checking: false,
        };

        let mut app = App::new(std::path::PathBuf::from("/tmp/neko-fields.db"));
        app.screen = Screen::Send(Box::new(st));
        app.set_viewport(105, 26);
        let out = render(&app, 105);

        // Columns are cells, not characters: a CJK label is two cells per
        // character, which is the whole reason this drifted.
        let col = |needle: &str| -> usize {
            let line = out
                .lines()
                .find(|l| l.contains(needle))
                .unwrap_or_else(|| panic!("{locale:?}: nothing contains {needle:?}\n{out}"));
            let at = line.find(needle).unwrap();
            unicode_width::UnicodeWidthStr::width(&line[..at])
        };

        let amount = col("0.5 ETH");
        let to = col(TO);
        let field = col("[");

        assert_eq!(
            amount, to,
            "{locale:?}: the amount and the destination start at different columns\n{out}"
        );
        assert_eq!(
            to, field,
            "{locale:?}: the password field does not line up with them\n{out}"
        );
    }
    neko_i18n::set_locale(neko_i18n::Locale::English);
}
