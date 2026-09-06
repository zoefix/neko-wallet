<div align="center">

# Neko Wallet

---

一个跑在终端里的自托管加密钱包。整个钱包就是**一个加密文件** ——
插U盘带走、放网盘、随便拷到哪里都行。用一个邮箱加一个密码解锁，
而这两样东西哪里都没有保存。

多链架构：TRON、Ethereum、BNB Chain、Polygon、Base、Arbitrum、Optimism、Avalanche、HyperEVM、Mantle、Linea、zkSync Era、Scroll、Aptos、Sui、Solana、Bitcoin、TON。

[English](README.md) | [简体中文](README.zh-CN.md) | [繁體中文](README.zh-TW.md) | [日本語](README.ja.md)

![Version](https://img.shields.io/badge/VERSION-v0.1.0-8A2BE2?style=for-the-badge&labelColor=444)
![Platform](https://img.shields.io/badge/PLATFORM-MACOS%20%7C%20LINUX%20%7C%20WINDOWS-00B5E2?style=for-the-badge&labelColor=444)
![Chains](https://img.shields.io/badge/CHAINS-TRON%20%7C%20ETH%20%7C%20BNB%20%7C%20SOL%20%7C%20BTC-1BC47D?style=for-the-badge&labelColor=444)
![Rust](https://img.shields.io/badge/RUST-1.86%2B-000000?style=for-the-badge&labelColor=444)
![Licence](https://img.shields.io/badge/LICENCE-MIT-F5A623?style=for-the-badge&labelColor=444)

</div>

## 这东西是干什么的

一个在终端里运行的加密货币钱包。私钥存在本地加密的 SQLite 金库里；没有账号要登录，
没有服务器保存任何东西，也不做同步。程序会联系的只有**你自己指定的链节点**，
以及 —— 仅当你自己填了 Key 时 —— 一个用于 BNB Chain 历史的索引服务。
不检查更新、不上报数据、没有任何统计，**也没有行情服务**：钱包列表里那个估值
是从链上的兑换池直接问出来的，所以显示它不花费任何隐私。它以 USDT 计价而不是美元，
并且界面上如实标注 —— 因为那才是实际问到的东西。

从存储结构往上就是按**钱包 → 链 → 资产**设计的。目前能用的是 TRON，
见[支持的链](#支持的链)。

金库是**一个自包含的文件**。拷到U盘上，回头再放回来，东西全在。
别人把这个文件拿走，拿到的是一堆密文。

```
$ neko-wallet
邮箱: zoe@example.com
密码:

  ⠸ 正在派生密钥... 这里慢是故意的

┌──────────────────────── 钱包 ────────────────────────┐
│ > 储蓄            TPZrDZ...  8.655007 TRX  7.00 USDT  │
│   日常            TWx3kQ...  0.000000 TRX  0.00 USDT  │
└───────────────────────────────────────────────────────┘
 n 新建   i 导入   Enter 打开   s 设置   q 退出
```

> [!WARNING]
> **没有任何找回途径。** 没有主助记词、没有重置链接、没有客服。
> 忘了邮箱或密码，钱包就没了 —— 你的处境和拿走文件的小偷完全一样。
> 这是设计，不是缺陷。请给 `.db` 留一份副本 —— 它是加密的，所以复制既便宜又安全。
> 在你考虑把助记词抄在纸上之前，请先读[备份](#备份)：那一步没有听起来那么无害。

---

## 安装

**macOS、Linux、WSL：**

```bash
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
```

**Windows PowerShell：**

```powershell
irm https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.ps1 | iex
```

**Windows CMD：**

```
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.cmd -o install.cmd && install.cmd
```

安装脚本下载适合你机器的版本，对照公布的校验和验证，放进 PATH，然后就结束了。
它不创建钱包，也不问你要任何东西。

把脚本管道给 shell 意味着执行那个 URL 提供的任何东西，介意的话请先读一遍 ——
[install.sh](install.sh) 大约两百行。`NEKO_WALLET_INSTALL_DIR` 改安装位置，
`NEKO_WALLET_NO_PATH=1` 则不动你的 shell 启动文件。

### 从源码编译

```bash
cargo install --git https://github.com/zoefix/neko-wallet
```

单个二进制，没有运行时依赖 —— SQLCipher 是编译进去的。Windows 上从源码编译需要
Perl 和 NASM，因为那边的 OpenSSL 要从源码构建。

---

## 上手

```bash
neko-wallet
```

首次运行会让你设置邮箱和密码，并创建金库。然后：

| 按键 | 作用 |
|---|---|
| `n` | 新建钱包（生成助记词，但**不显示**） |
| `i` | 从助记词或私钥导入 |
| `Enter` | 打开 → 链 → 资产 |
| `y` | 复制地址 |
| `s` | 转账 |
| `t` | 交易记录 |
| `m` | 查看这个钱包的助记词 |
| `<` `>` | 切换语言 |
| `L` | 立即锁定 |

新建钱包时不会给你看助记词。你要在之后主动按 `m` 去看 —— 而那会再问一次密码。

---

## 支持的链

一个钱包的那一份助记词，覆盖它下面的所有链 —— BIP44 派生路径就是干这个用的。
加一条链，永远不意味着新钱包、新助记词、或者多一样要备份的东西。

| 链 | 状态 | 资产 |
|---|---|---|
| TRON | 可用 | TRX、USDT (TRC20) |
| BNB Chain | 可用 | BNB、USDT (BEP20) |
| Solana | 可用 | SOL、USDT (SPL) |
| Bitcoin | 可用 | BTC |
| Ethereum | 可用 | ETH、USDT (ERC20) |
| Polygon | 可用 | POL、USDT (ERC20) |
| Base | 可用 | ETH、USDC (ERC20) |
| Arbitrum | 可用 | ETH、USDT (ERC20) |
| Optimism | 可用 | ETH、USDT (ERC20) |
| Avalanche (AVAX C-Chain) | 可用 | AVAX、USDT (ERC20) |
| HyperEVM | 可用 | HYPE、USDT (ERC20) |
| Mantle | 可用 | MNT、USDC (ERC20) |
| Linea | 可用 | ETH、USDC (ERC20) |
| zkSync Era | 可用 | ETH、USDC (ERC20) |
| Scroll | 可用 | ETH、USDT (ERC20) |
| Aptos | 可用 | APT、USDT (fungible asset) |
| Sui | 可用 | SUI、USDC (coin objects) |
| TON | 可用 | GRAM、USDT (jetton) |

Polygon 也用同一个币种编号，所以同一句助记词在三条 EVM 链上是**同一个地址**。
它有两件自己的事。它的币叫 **POL**，2024 年 9 月从 MATIC 改名 —— 这一点是链自己说的，
也是这个钱包判断的依据：包装后的原生代币合约报的是 `WPOL`。它的 USDT 合约自称
**`USDT0`**，因为 Tether 把 Polygon 上的供应迁到了全链版本。它就是大家说的那个 USDT，
钱包也照 USDT 显示；签名前拿去跟链核对的，是合约实际叫什么。Polygon 的历史记录走
**Blockscout**，因为 NodeReal 不索引这条链。它不需要密钥，所以是这里唯一一个不用你
开口就会去联系的索引服务 —— 跟比特币那边用 Esplora 是同一笔交易，理由也一样：
`eth_getLogs` 一次上限一万个区块，而 Polygon 1.5 秒一个块，那只有四个小时，
何况原生转账根本不产生日志。

设置里的 **Etherscan API 密钥**是可选的，一把就够这里所有 EVM 链用；填了就优先用它。

Base 是第四条 EVM 链，地址还是同一个。它的币是 ETH —— 跟 Ethereum 是同一种资产，
两条链上的转账只靠 chain id 区分。它做不到的是给这个币定价：它自己的 Uniswap V2
WETH/USDT 池子确实存在，但**总共只有十七美元**，去问它一个 ETH 值多少，答案是十七。
Base 的流动性在 Aerodrome 和 Uniswap V3 上，这两个都不是同一套接口，所以价格改从
Ethereum 的池子读 —— 同一种资产，而且那条链本来就在联系。这跟 BTC 的做法是一样的。
历史记录跟 Polygon 一样走 Blockscout。

还有一件事 Base 只跟 Optimism 共有：**手续费不等于 gas × 单价**。OP-stack rollup 会把
交易写到 Ethereum 上，这笔钱由发送方在 L2 gas 之外另付，而且 `op-geth` 把它算进余额
检查里 —— 所以一个把手续费当成 gas × 单价的钱包会正好差这么多，「全部转出」会被节点以
`have … want …` 拒掉。这个金额是拿真正要签名的那串字节去问链上的 L1 gas 价格预言机
取的，在确认页单列一行。它不退。

链上按**压缩后**的体积收费，所以「签名还没生成、拿什么字节占位」不是细节而是钱：一串
重复的填充会被压没，预留就不够。拿两条链的预言机实测，这个缺口在 1.33 亿到 6.33 亿 wei
之间，取决于是哪条链、转的是不是代币。

Base 也是这里唯一一条稳定币是 **USDC 而不是 USDT** 的链。这不是偏好：Tether 在 Base
上的合约总共 2380 万，Circle 的 USDC 是 42 亿；币安列出的 USDT 可提网络有 19 个，
里面没有 Base —— 它在 Base 上只支持 ETH 和 USDC。放一行 USDT 在那儿，是一行谁也填
不满的余额。所以「每条链一格代币」现在是「这条链的稳定币」，是哪个就写哪个。

Arbitrum One 是第五条 EVM 链、第二条 rollup，而它收 L1 费用的方式正好相反：Nitro 把
写到 Ethereum 的成本折进了 gas **估算**里 —— 一笔普通转账估出来是 21,302 而不是
21,000 —— 所以 `gas_limit × 单价` 已经包含了它，不需要另外预留。Base 需要那笔预留，
这条链必须没有；在这里凭空扣一笔，每次「全部转出」都会剩下灰尘。它的币也是 ETH，
价格同样从 Ethereum 读，理由跟 Base 一样：它自己的池子只有大约三万美元，报价慢了 14%。
它的 USDT 是真的 —— 8.35 亿，币安也能提过去 —— 只是合约自称 `USD₮0`，那个 T 的位置
是图格里克符号。这个名字只拿去跟链核对，从不显示。

Optimism 是第六条 EVM 链、第三条 rollup，也是「是不是 rollup」这个问题失效的地方。它跟
Base 一样是 OP-stack，所以要另外预留 L1 费用、要去问预言机；Arbitrum 同样是 rollup，
却绝不能问。想知道是哪一类，问链就行：一笔普通转账在两条 OP-stack 链上都正好估 21,000
gas，在 Arbitrum 上估 21,422 —— 那个多出来的部分就是已经折进去的写入成本。

这条链上有两样东西照抄邻居就是错的。它的 Uniswap V2 路由地址跟 Base、Arbitrum **不一样**
—— 那两条链共用一个地址，而那个地址在 Optimism 上没有代码，抄过来不会报错，只会什么都
不返回，然后价格就悄悄没了。另外，不像 Polygon 的 `USDT0` 和 Arbitrum 的 `USD₮0`，它的
USDT 合约老老实实就叫 `USDT`：2.235 亿，地址跟币安自己列的提币地址一致。它的币是 ETH，
价格从 Ethereum 读，因为它自己的两个 V2 池子分别只有十五美元和五百多美元，报出来的
ETH 价是 7.55 和 264。历史记录走 Blockscout，只是这家开在自己的域名上。NodeReal 有这条
链的 RPC，但没有对应的索引，所以历史不走它。

这一批六条链一起加进来，分类的依据不是它们叫什么，而是它们怎么收 L1 的钱：

| 链 | 币 | 稳定币 | L1 费用 | 币价来源 |
|---|---|---|---|---|
| Avalanche | AVAX | USDT（`USDt`） | — | 自己的池子 |
| HyperEVM | HYPE | USDT（`USD₮0`） | — | **没有** |
| Mantle | MNT | USDC | 在 gas 之外另收 | **没有** |
| Linea | ETH | USDC | — | Ethereum |
| zkSync Era | ETH | USDC | 折进 gas 里 | Ethereum |
| Scroll | ETH | USDT | 另收，**而且地址不一样** | Ethereum |

**Scroll 收 L1 用的是它自己的地址。** 它的 `L1GasPriceOracle` 在 `0x53..02`，
3,782 字节；Base、Optimism、Mantle 共用的 OP-stack 预部署在 `0x42..0F`，2,055 字节。
两者认同一个 `getL1Fee(bytes)`，所以一个字段能覆盖两种设计——但如果去 OP-stack 那个
地址上找 Scroll 的，会找到一片空白，然后一分钱都不预留。

**zkSync Era 一笔普通转账估出来是十七万八千 gas**，不是 21,000。它跟 Arbitrum 一样把
写到 Ethereum 的成本折进了 gas，只是折得更彻底。它不另外收费。一个"觉得这数不对、
把它改回 21,000"的钱包，造出来的交易根本进不了块。

**有两条链故意不显示币价。** HYPE 和 MNT 在这个钱包接触的其它链上都不存在，而它们
自己的 Uniswap V2 池子只有大约 1,100 美元和 400 美元。这么小的池子不会报错——它会
给你一个数。HyperEVM 那个池子的现价其实准到 0.1% 以内，但问它"一个币值多少"，
答案低了 13%，因为一个币就是池子的六分之一。所以估值那一列直说"不知道"，这是真话；
总比给一个错得看不出来的数强。

**Avalanche 和 Mantle 的历史记录走 Routescan**，就是跑 Snowtrace 的那家，不需要密钥。
这两条是它在这里唯一支持的链——对另外十条它都回 `chain not supported`——而这也是这两条链
唯一的索引来源：NodeReal 两条都不覆盖，也都没有 Blockscout 实例。**Linea 的 Blockscout 开在一个它自己的浏览器都不写出来的地址上**——`explorer.linea.build`
是个前端，对所有 API 路径都返回 HTML，真正的后端在 `api-explorer.linea.build`。
这样一来只剩 **HyperEVM** 一条链的历史记录需要你自己的 Etherscan key。它的 Blockscout
原本是 Hyperscan，现在已经跳转到宣传页；Hyperliquid 自己的浏览器接口只列出一个地址
**转出**的交易，那正是这个钱包已经犯过一次、不会再犯的「半部历史」；而 `eth_getLogs`
在这条链上一次只能查 1000 个区块，大约十七分钟。Etherscan V2 是覆盖它的，所以界面上说的是
「去加一个 key」，而不是「这里没有索引」——这是两回事，后者是假话。

**Avalanche 的名字里带着网络**——`Avalanche (AVAX C-Chain)`——因为它其实是三条链，
而地址本身说不清是哪一条。交易所三条都提供，其中两条接受 `0x` 地址：C-Chain（就是这条），
还有 BNB Chain——它照单全收同样的二十个字节，然后把币送到这个钱包的 Avalanche 页面
永远不会去看的地方。X-Chain 会当场拒绝这个地址，是三者里唯一自己会拦的。

**同样三个字母，四种写法。** Tether 的合约在 Ethereum 和 Scroll 上自称 `USDT`，
在 Polygon 上是 `USDT0`，在 Arbitrum 和 HyperEVM 上是带图格里克符号的 `USD₮0`，
在 Avalanche 上是小写 t 的 `USDt`。每一个都会在签名前跟链核对，而且一个都不会显示到
屏幕上。

这里有两条链既不是 EVM，彼此也完全不一样。

**Aptos** 把 coin 和 *fungible asset* 分成两套东西，而它的 USDT 属于后者。两者的入口
函数不同、查余额的方法也不同，拿一套去操作另一套不会转错金额——交易直接 abort。它的
地址是 `sha3_256(公钥 || scheme)`，所以同一把钥匙换个签名方案就是另一个账户。

交易编码不是照着文档写的，是跟链自己的编码器对过的：Aptos 提供
`/transactions/encode_submission`，把交易交给它、它返回签名要覆盖的字节。这个钱包
生成的字节跟它**逐字节相同**，两种转账分别 197 和 297 字节。这件事很要紧，因为 BCS
不带类型——长度字节放错位置不会得到一个损坏的交易，而是得到另一个交易，并且被正确
地签了名。

它的手续费显示的是**上限**而不是估算。Aptos 可以精确计价，但前提是拿到发送方的公钥；
报价的时候这个钱包没有——密钥要到最后一步才推导，而地址是公钥的哈希、反推不回去。
没用掉的 gas 不收费。

**Sui 上没有"账户里有个数字"这回事。** 币是**对象**，每个都有 id、版本和摘要；余额是
它们的和，转账花掉的是其中某几个。这让它成为继 Bitcoin 之后第二条"发送是一次挑选而
不是一次减法"的链，也让"版本过期"变成拒绝而不是少转一点。它也没有"转账"这条指令：
每笔交易都是一串命令，一次付款是先切一刀、再递过去。

它的地址是 `blake2b256(scheme || 公钥)`——注意顺序，跟 Aptos 正好相反。两条链、两把
Ed25519 钥匙、两个 32 字节地址，把两个操作数调换一下，得到的是一个看起来完全正常、
却没有人握有私钥的账户。验证方法是从主网拿真实交易，从签名里读出签名者的公钥，再重新
算一遍地址。

手续费来自对这笔交易本身的 dry run，这同时也证明了字节是对的：链自己的解析器必须
读懂它才能给它定价。有一个后果值得知道——Sui 会退还它收取的一部分存储费，而把币对象
合并起来正好释放存储，所以一笔把三十个对象折在一起的转账**可能一分钱都不花**。

一个提醒：Sui 官方的公共全节点已经**停用**了 JSON-RPC，对所有方法都回
"Method not found"。这里默认用的是另一个还在提供它的节点。

TON 在这里是唯一一条**地址不是从密钥算出来的**链。TON 上的钱包本身就是一个智能合约，
地址是这个合约初始代码和存储的哈希。由此带来的两件事都摆在界面上，而不是等你自己撞上：
钱包转出的第一笔要把合约代码一起带出去，因为在那之前这个地址只是有余额、却没有任何
能动它的东西；转 USDT 需要 GRAM，因为代币转账是两个合约之间发消息，每一跳都得靠随
消息带上的钱才跑得动 —— 其中大部分会退回来。转账界面把这笔随带金额和手续费分开显示，
而不是加在一起。

因为地址取决于钱包合约，**同一句助记词，这里的 TON 地址不一定和 Tonkeeper 或 Telegram
里的一致** —— 除非它们也用 v4R2 和标准的 subwallet id，这是常见默认值，但不是保证。
币叫 GRAM：2026 年 6 月 15 日从 Toncoin 改回了它在 Telegram 2018 年白皮书里的名字。
改的只是代号 —— 网络还是 TON，地址和余额都没变。

链相关的代码只集中在一个 crate 里。密钥派生、存储、加密和界面都是共用且与链无关的，
数据库从第一版迁移起就带着一张有 SLIP-44 币种编号的 `chains` 表。
本文里关于能量、带宽和手续费估算的部分是 TRON 特有的；其余内容适用于任何链。

---

## 你的钱包就是一个文件

这一节值得看懂，因为备份和换机器全靠它。

```
neko-wallet.db     ← 这就是钱包。全部。
```

一个文件。没有 `-wal`、没有 `-shm`、没有任何附属状态 —— 数据库跑在
`journal_mode = DELETE` 下就是为了这个：拷贝这个文件等于拷贝了全部。
它整体加密，所以可以放在那些你绝不会放私钥的地方：

```bash
cp ~/.local/bin/neko-wallet.db /Volumes/USB/            # U盘
cp ~/.local/bin/neko-wallet.db ~/Dropbox/backup/        # 网盘
```

要使用别处的金库：

```bash
neko-wallet set db /Volumes/USB/neko-wallet.db   # 从此记住
neko-wallet --db /Volumes/USB/neko-wallet.db     # 只这一次
neko-wallet --where-db                           # 现在打开的是哪个？
neko-wallet unset db                             # 恢复默认查找
```

`set db` 不需要邮箱和密码：它只记一个路径，别的什么都不做。
路径不存在时它会拒绝，除非你加 `--new` —— 因为路径不存在通常就是打错了，
而后果是一个看起来像「我的钱包不见了」的首次设置界面。

打开哪个文件的顺序：`--db` → `$NEKO_WALLET_DB` → 保存的设置 →
可执行文件同目录 → 系统数据目录。

---

## 备份

有两种备份，而它们的失效方式**正好相反**。在你真正需要它们之前，值得先搞清楚谁是谁。

### `.db` 文件 —— 该依靠的是这个

整体加密。放在U盘或网盘上的副本就是一坨密文：捡到的人依然需要你的邮箱和密码。
所以尽管多拷、经常拷。

```bash
cp ~/.local/bin/neko-wallet.db /Volumes/USB/
```

它挡不住的是**你自己忘记密码**。到那一步，这个文件对你的用处和对小偷的用处完全一样：没有。

### 助记词 —— 最后的退路，不是日常备份

> [!CAUTION]
> **不推荐把这十二个词抄在纸上当作主要备份。**
> 纸上的助记词是**明文的持有型凭证**：任何人只要读到它，就能立刻把你的钱转走 ——
> 入室的小偷、来家里的客人、从你肩后拍的一张照片，或者干脆直接从你手里抢走。
> 它不会问密码，也不会有任何提示。`.db` 文件完全没有这个性质，
> 这正是它才是该依靠的那份备份的原因。

如果你确实要抄下来，那张纸就等价于钱包里的钱，也该按同样的方式保管：

- 保险箱或银行保管箱。不是抽屉，不是笔记本，不是书页之间。
- **绝对不要**拍照、存进云笔记、发给自己、或者输入任何设备。
  只要会同步，它就已经脱离你的控制了。
- 金额值得的话，拆成两份分开存放，让一次入室不足以拿走全部。

唯一能让「助记词被偷走也没用」的东西是 BIP39 passphrase —— 一个只存在你脑子里的
「第 25 个词」。neko-wallet 在**导入**钱包时支持它，但生成新钱包时还没有提供设置入口。

---

## 转账

确认方式不是按一个键，而是**手打收款地址的最后 6 个字符**。

```
   收款  TPZrDZ TUWQqqUTVRxAmSdQyGXSSg AUyyk4
         ^^^^^^                        ^^^^^^

   请输入最后 6 个字符以确认：
         [ AUyyk_ ]  5/6
```

剪贴板劫持恶意软件在你粘贴时替换收款地址，是命令行钱包最主要的真实损失来源。
按 `y` 拦不住它 —— 因为你看的是你**以为**自己粘贴进来的东西。
手打末尾几位是唯一强迫你的眼睛落到即将被签名的那串字节上的动作。

然后要再输入一次完整密码。一个人走开了、还开着的终端，不该足以把钱转走。

**手续费会拆开显示**，因为 TRON 没有固定手续费：转账消耗带宽，合约调用消耗能量，
只有你的账户覆盖不了的那部分才会烧 TRX。能量是针对你这一笔交易向链上模拟出来的，
从不靠猜 —— 同样一笔 USDT，转给从没持有过 USDT 的地址大约要贵一倍。

### 地址投毒

攻击者会从一个长得很像你常用地址的地方给你转粉尘，等你从交易记录里复制错。
三重防御：

- 粉尘交易默认隐藏。
- **任何地址都不缩写显示。** 一旦渲染成 `TPZr…yyk4`，冒牌货和真货就没法区分了。
- 收款地址如果**像但不等于**你交易记录里的某个地址，会明确警告。

---

## 助记词

按 `m` 查看某个钱包的十二个词。在那之前：

- 即使金库已经解锁，也会**完整重跑一遍** Argon2id 验证你的密码。
- 词是遮住的，方向键**一次只揭开一个**。截屏或 `tmux capture-pane`
  漏掉的是十二分之一，不是全部。
- 60 秒后自动隐藏。
- **复制不是「被拒绝」，而是根本不存在。** 那个界面到任何剪贴板后端之间没有代码路径，
  所以不存在一个将来可能被 bug 反转的判断。

界面上也会写清楚这些都挡不住什么：相机、背后的人、`script`、录屏软件，
以及已经在你机器上的木马。一个声称终端能守住屏幕上的秘密的钱包，是在撒谎。

在你把这些词抄到纸上之前，请先读[备份](#备份) —— 抄下来的助记词是持有型凭证，
`.db` 文件才是更安全的那份备份。

---

## 语言

English、简体中文、繁體中文、日本語。按系统语言自动检测，随时用设置里的
`<` `>` 切换，并按金库记住。

每种语言都用它自己的文字列出，这样看不懂当前语言的人也能找到出路。
翻译在编译期校验：缺 key、`%{占位符}` 对不上、或出现歧义宽度字符，
都会让构建失败，而不是流到一个正拿着钱的用户面前。

---

## 它是怎么工作的

### 密钥层级

```
邮箱+密码 ─Argon2id─► stretched ─HKDF─┬─► 文件密钥 ──► SQLCipher（整库）
                                      └─► KEK ──► 解封 MK（32 字节随机）
                                                   │
                                              HKDF ├─► k_data   字段级 AEAD
                                                   └─► k_index  盲索引
```

每个钱包各有一份独立的 BIP39 助记词，用 `k_data` 加密后存库。
`MK` 是纯随机的、与任何助记词无关 —— 这正是为什么**没有主助记词**。
改密码只需重新封装那 32 字节，不用重新加密任何业务数据，而且是崩溃安全的
（切换完成前两份封装都保留）。

### 两层加密

整个文件由 SQLCipher 加密（AES-256-CBC + HMAC-SHA512）。在这之上，
那些一旦泄露就等于丢钱的东西还各自套了一层独立密钥的 XChaCha20-Poly1305：

| 字段 | 第二层 |
|---|---|
| 助记词熵 | 有 |
| 私钥 | 有 |
| 钱包名称 | 有 —— 「公司备用金」本身就是情报 |
| TronGrid API Key | 有 |
| 地址、交易、余额 | 无 —— 链上本就公开，且需要索引排序 |

每段密文的 AAD 都绑定了表名、列名、行 ID 和密钥版本，所以密文行之间无法掉包。

### 盐存在哪

一个自包含的文件必须只靠它自己就能解密，但你得**先拿到盐**才能解密。
SQLCipher 把它那 16 字节的盐放在文件最前面、明文存储 —— 所以我们把那个位置
当成自己的文件头用：格式版本、KDF 档位，加 14 字节本库唯一的随机数。

把档位放在未认证的文件头里是安全的：改了它就会算出不一样的密钥，文件直接打不开。
降级攻击自己把自己废掉了。

### 计算成本

Argon2id，安装时在你的机器上实测标定 —— 128 MiB / t=4、256 MiB / t=3
或 1 GiB / t=4。档位号写在文件头里，所以在快机器上建的库拿到慢机器照样能开。

密码必须过 70 bit 的估算熵，取「字符集法」和「模式法」两者中**较小**的那个，
所以 `MyWallet2026!` 会被算成它实际的样子。

### 交易在本地构造

绝不调用 `/wallet/createtransaction`。节点只提供区块引用；
即将被签名的字节由本地用手写的 protobuf 编码器组装，签完立刻从签名恢复公钥、
断言地址一致。一个恶意节点没法塞给你一笔转给他自己的交易再骗你签字。

---

## 升级

再跑一次安装脚本。

```bash
curl -fsSL https://raw.githubusercontent.com/zoefix/neko-wallet/main/install.sh | sh
```

它替换二进制，绝不碰你的 `.db` —— 结束时会明确告诉你这一点。

**这里故意不做自动更新。** 一个能替换自己可执行文件的钱包，就等于在保管私钥的机器上
开了一条远程代码通道，再多的签名验证也不会让这条通道比「根本没有」更小。
CI 会强制这一点：依赖树里一旦重新出现自替换相关的 crate，
或者程序多出白名单以外的联网目标，构建就会失败。
白名单必须手动修改才能扩大，目前只有：三条链的节点、可选的 BNB Chain 历史索引服务，
以及浏览器链接（只复制给你，钱包自己从不访问）。

---

## 防得住什么，防不住什么

**防得住：**

- `.db` 文件被拷走、同步到云端泄露、或从丢弃的硬盘里被恢复 —— 没有密码它就是密文。
- 对密文逐行逐列的篡改。
- 篡改 KDF 成本参数让文件变得更容易爆破。
- 恶意或被攻破的节点，试图骗你签一笔转给别人的交易。

**防不住：**

- **忘记密码。** 永久、彻底丢失。这就是「零恢复」的含义。
- 你运行它的机器上的键盘记录器或远控木马。**它不是硬件钱包**：
  金库打开期间，私钥就在这台电脑的内存里，终端程序改变不了这一点。
- 对已解锁进程的内存转储。密钥会做归零处理、尽量不放进 `String`，但操作系统层面挡不住。
- 有人从你屏幕上读走助记词。
- 弱密码。`Password123` 配 Argon2id，它依然是 `Password123`。

---

## 开发

```bash
cargo test --workspace                              # 275 个测试
cargo clippy --workspace --all-targets -- -D warnings
```

密码学、密钥派生和 TRON 交易编码都对照 `vectors/` 里冻结的跨语言测试向量校验 ——
逐字节比对，所以任何改动只要让派生地址或交易 ID 变了，立刻就会失败。

```
neko-crypto   Argon2id / XChaCha20-Poly1305 / HKDF。无 IO、无 SQL、无 async
neko-vault    密钥层级、KDF 档位、密码策略、归一化
neko-store    SQLCipher、迁移、字段级信封（永不派生密钥）
neko-hd       BIP39 / BIP32 / BIP44 与 SLIP-0010；TRON、EVM、Solana 地址编解码
neko-tron     仅 TRON：protobuf、交易构造与签名、节点客户端
neko-evm      五条 EVM 链：RLP、EIP-155/1559 签名、ABI、JSON-RPC、rollup 手续费
neko-solana   Solana：Ed25519、交易编码、代币账户、集群 RPC
neko-btc      Bitcoin：bech32、隔离见证签名、选币、Esplora
neko-ton      TON：cell 与 BoC、钱包合约 v4R2、jetton、toncenter
neko-core     界面唯一对话的门面
neko-i18n     编译期校验的翻译表
neko-tui      ratatui 界面
```

---

## 许可

MIT。见 [LICENSE](LICENSE)。
