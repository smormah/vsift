//! Fetching one video from a direct HTTPS link into a session's source slot
//! (ADR 0028, #407).
//!
//! The network is used here and only here, and only for a link a caller gave
//! `ingest --url`. One `GET` per fetch, at most three redirects, HTTPS on port
//! 443 only, no credentials, no cookies, no referer, a neutral user agent and
//! an identity body; the size and time are bounded as bytes arrive, and the
//! first twelve bytes must be an admitted container's before more is taken.
//!
//! **The address policy is applied where the connection is made.** A name is
//! resolved by [`PublicAddressResolver`], which hands the connector only
//! public unicast addresses and refuses a name with any other address, so the
//! address checked is the address connected to: DNS rebinding cannot reach a
//! private address. A host written as an address is checked by the link and
//! redirect policy ([`check_link`]), because it is never resolved. Through a
//! proxy from the environment the proxy resolves the name; then only
//! addresses and names for internal hosts are refused, on the link and on
//! every redirect, and the origin says `proxy_resolved` (known limit L-208).
//! A proxy variable that is set but unusable is refused, never ignored.
//!
//! Cancellation and the deadline are observed while the fetch waits for the
//! response and for each chunk. Writing a chunk and the final flush to disk
//! are not interrupted, so on a slow disk a cancellation or the deadline takes
//! effect once they finish.
//!
//! Failures carry a typed [`SourceFetchReason`], never the server's text, the
//! link or proxy credentials.

use std::{
    error::Error,
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr},
    sync::{
        Arc,
        atomic::{AtomicU8, Ordering},
    },
    time::Duration,
};

use reqwest::{
    Client, StatusCode, Url,
    header::{ACCEPT, ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_RANGE, CONTENT_TYPE},
    redirect::Policy,
};
use sha2::{Digest, Sha256};
use tokio::time::{Instant, timeout_at};
use vsift_application::StageCancellation;
use vsift_domain::{
    AddressCheck, DownloadLimits, HostName, MAX_SOURCE_REDIRECTS, Sha256Hex, SourceFetchReason,
    SourceLink,
};

const CONNECT_DEADLINE: Duration = Duration::from_secs(15);
/// No byte for this long stops the transfer as stalled.
const STALL_DEADLINE: Duration = Duration::from_secs(30);
/// How often a wait for the next chunk looks at the caller's cancellation.
const CANCEL_POLL: Duration = Duration::from_millis(200);
/// The one port a link may name.
const HTTPS_PORT: u16 = 443;
/// How much room a body of unknown length must find before the next block of
/// it is written: checked before the first byte and again as each block
/// begins, so a small video sent without a length needs room for itself,
/// not for the whole download limit.
const ROOM_STEP: u64 = 64 * 1024 * 1024;
/// The bytes that identify an admitted container (ISO base media `ftyp` at
/// offset 4, Matroska's EBML header at 0).
pub(crate) const MAGIC_BYTES: usize = 12;
/// How hyper-util's private `TunnelError::ProxyAuthRequired` ends its text;
/// the same marker as the managed transfer's (L-088).
const PROXY_AUTH_REQUIRED: &str = "proxy authorization required";
/// Name suffixes that are internal by convention, refused when a proxy
/// resolves names (`VSift` then cannot see the address).
const INTERNAL_SUFFIXES: [&str; 7] = [
    ".localhost",
    ".local",
    ".internal",
    ".lan",
    ".home.arpa",
    ".corp",
    ".intranet",
];

/// The user agent of every link fetch: the product and its purpose only,
/// never a user, a host or contact details.
fn user_agent() -> String {
    format!("VSift/{} link-ingest", env!("CARGO_PKG_VERSION"))
}

/// A refusal raised inside the transport (the resolver or the redirect
/// policy), carried through reqwest's error chain to keep its reason.
#[derive(Debug)]
struct PolicyRefusal(SourceFetchReason);

impl fmt::Display for PolicyRefusal {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

impl Error for PolicyRefusal {}

/// Whether `address` is a public unicast address a link may reach.
///
/// Refused: IPv4 `0.0.0.0/8`, `10/8`, `100.64/10` (shared, carrier-grade
/// NAT), `127/8`, `169.254/16` (link-local, the cloud metadata address),
/// `172.16/12`, `192.0.0/24`, `192.0.2/24`, `192.88.99/24`, `192.168/16`,
/// `198.18/15`, `198.51.100/24`, `203.0.113/24`, `224/4` and `240/4`
/// (broadcast included). IPv6 outside `2000::/3`, and inside it Teredo
/// (`2001::/32`), the benchmarking, documentation and ORCHID ranges and
/// `3fff::/20`; a 6to4 (`2002::/16`) or well-known NAT64 (`64:ff9b::/96`)
/// address is judged by the IPv4 address it embeds; an IPv4-mapped or
/// IPv4-compatible address is refused.
#[must_use]
pub fn is_public_address(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => is_public_v4(v4),
        IpAddr::V6(v6) => is_public_v6(v6),
    }
}

fn is_public_v4(address: Ipv4Addr) -> bool {
    let [a, b, c, _] = address.octets();
    !(a == 0
        || a == 10
        || (a == 100 && (64..=127).contains(&b))
        || a == 127
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 0 && (c == 0 || c == 2))
        || (a == 192 && b == 88 && c == 99)
        || (a == 192 && b == 168)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113)
        || a >= 224)
}

fn is_public_v6(address: Ipv6Addr) -> bool {
    let segments = address.segments();
    if segments[0] == 0x0064 && segments[1] == 0xff9b && segments[2..6] == [0, 0, 0, 0] {
        // NAT64 well-known prefix: the IPv4 address in the last 32 bits.
        return is_public_v4(embedded_v4(segments[6], segments[7]));
    }
    if segments[0] & 0xe000 != 0x2000 {
        return false;
    }
    if segments[0] == 0x2002 {
        // 6to4: the IPv4 address in bits 16 to 47.
        return is_public_v4(embedded_v4(segments[1], segments[2]));
    }
    let refused_2001 = segments[0] == 0x2001
        && (segments[1] == 0x0000
            || segments[1] == 0x0db8
            || (segments[1] == 0x0002 && segments[2] == 0)
            || (0x0010..=0x002f).contains(&segments[1]));
    let documentation_3fff = segments[0] == 0x3fff && segments[1] & 0xf000 == 0;
    !(refused_2001 || documentation_3fff)
}

fn embedded_v4(high: u16, low: u16) -> Ipv4Addr {
    let [a, b] = high.to_be_bytes();
    let [c, d] = low.to_be_bytes();
    Ipv4Addr::new(a, b, c, d)
}

/// Whether a host name is internal by convention: one label, `localhost` or
/// one of the internal suffixes (`.localhost`, `.local`, `.internal`, `.lan`,
/// `.home.arpa`, `.corp`, `.intranet`).
#[must_use]
pub fn is_internal_name(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    !host.contains('.')
        || host == "localhost"
        || INTERNAL_SUFFIXES
            .iter()
            .any(|suffix| host.ends_with(suffix))
}

/// Whether a proxy must not be asked for `url`: its host is a name for an
/// internal host ([`is_internal_name`]). Applied to the link and to every
/// redirect when a proxy resolves names, because `VSift` then cannot see the
/// address; a host written as an address is judged by [`check_link`].
fn internal_through_proxy(url: &Url) -> bool {
    url.domain().is_some_and(is_internal_name)
}

/// What the link and redirect policy allows besides public HTTPS on 443:
/// production allows nothing more; the development-only test route allows
/// its local server's port and loopback address.
#[derive(Clone, Copy, Debug, Default)]
struct Allowance {
    test_port: Option<u16>,
    test_loopback: bool,
}

impl Allowance {
    fn address(self, address: IpAddr) -> bool {
        is_public_address(address)
            || (self.test_loopback && address == IpAddr::V4(Ipv4Addr::LOCALHOST))
    }
}

/// Checks a link or a redirect's target: HTTPS, no user name or password,
/// port 443, a host, and a public address when the host is written as one.
///
/// # Errors
///
/// The typed reason of the first rule broken.
fn check_link(url: &Url, allowance: Allowance) -> Result<(), SourceFetchReason> {
    if url.scheme() != "https" {
        return Err(SourceFetchReason::NotHttps);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(SourceFetchReason::CredentialsInLink);
    }
    match url.port() {
        None | Some(HTTPS_PORT) => {}
        Some(port) if Some(port) == allowance.test_port => {}
        Some(_) => return Err(SourceFetchReason::PortNotAllowed),
    }
    let host = url.host_str().ok_or(SourceFetchReason::NotHttps)?;
    match literal_address(host) {
        Some(address) if !allowance.address(address) => Err(SourceFetchReason::AddressRefused),
        _ => Ok(()),
    }
}

/// The address a host is written as, if it is one (`[::1]` or `127.0.0.1`;
/// the URL parser has already written any IPv4 form in dotted decimal).
fn literal_address(host: &str) -> Option<IpAddr> {
    host.strip_prefix('[')
        .and_then(|rest| rest.strip_suffix(']'))
        .map_or_else(
            || host.parse::<Ipv4Addr>().ok().map(IpAddr::V4),
            |inner| inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6),
        )
}

/// Resolves names for a link fetch and hands the connector only public
/// addresses: a name with any address that is not public is refused whole.
struct PublicAddressResolver {
    allowance: Allowance,
    #[cfg(any(test, feature = "install-test-hooks"))]
    answers: Option<Arc<TestAnswers>>,
}

impl reqwest::dns::Resolve for PublicAddressResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let host = name.as_str().to_owned();
        let allowance = self.allowance;
        #[cfg(any(test, feature = "install-test-hooks"))]
        if let Some(answers) = &self.answers {
            let addresses = answers.next(&host);
            return Box::pin(std::future::ready(screen(addresses, allowance)));
        }
        Box::pin(async move {
            let addresses = lookup(&host).await?;
            screen(addresses, allowance)
        })
    }
}

/// Every address of `host`, from the system resolver.
async fn lookup(host: &str) -> Result<Vec<IpAddr>, Box<dyn Error + Send + Sync>> {
    #[cfg(debug_assertions)]
    if !crate::publisher_artifact_transfer::development_network_allowed() {
        // Development builds resolve no name (the P13 PR 4 network guard).
        return Err(Box::new(PolicyRefusal(SourceFetchReason::Offline)));
    }
    let resolved = tokio::net::lookup_host((host, HTTPS_PORT))
        .await
        .map_err(|_| {
            Box::new(PolicyRefusal(SourceFetchReason::Offline)) as Box<dyn Error + Send + Sync>
        })?;
    Ok(resolved.map(|address| address.ip()).collect())
}

/// The connector's addresses for one resolved name, or the refusal.
fn screen(
    addresses: Vec<IpAddr>,
    allowance: Allowance,
) -> Result<reqwest::dns::Addrs, Box<dyn Error + Send + Sync>> {
    if addresses.is_empty() {
        return Err(Box::new(PolicyRefusal(SourceFetchReason::Offline)));
    }
    if !addresses.iter().all(|address| allowance.address(*address)) {
        return Err(Box::new(PolicyRefusal(SourceFetchReason::AddressRefused)));
    }
    let port = allowance.test_port.unwrap_or(HTTPS_PORT);
    Ok(Box::new(
        addresses
            .into_iter()
            .map(move |address| SocketAddr::new(address, port))
            .collect::<Vec<_>>()
            .into_iter(),
    ))
}

/// The proxy the environment names for `url`, if any: `HTTPS_PROXY` (or
/// `https_proxy`), else `ALL_PROXY` (or `all_proxy`), unless `NO_PROXY` (or
/// `no_proxy`) lists the host. `VSift` decides this itself, so it knows when
/// the proxy, not `VSift`, resolves the name.
///
/// # Errors
///
/// [`SourceFetchReason::ProxyUnusable`] when a proxy is named for `url` but
/// is not one `VSift` can use ([`parse_proxy`]).
fn environment_proxy(url: &Url) -> Result<Option<Url>, SourceFetchReason> {
    proxy_from_variables(url, |name| std::env::var(name).ok())
}

/// [`environment_proxy`] with the variables read by `variable`, so the rule
/// is tested without changing the process environment.
///
/// `NO_PROXY` entries are compared by host name only: an entry is the host
/// itself or a domain it is under (a leading `.` is optional), or `*` for
/// every host. Wildcards inside an entry, addresses, address ranges and
/// ports are not read (the troubleshooting guide says so).
fn proxy_from_variables(
    url: &Url,
    variable: impl Fn(&str) -> Option<String>,
) -> Result<Option<Url>, SourceFetchReason> {
    let read = |names: [&str; 2]| {
        names
            .into_iter()
            .find_map(|name| variable(name).filter(|value| !value.trim().is_empty()))
    };
    let Some(proxy) =
        read(["HTTPS_PROXY", "https_proxy"]).or_else(|| read(["ALL_PROXY", "all_proxy"]))
    else {
        return Ok(None);
    };
    let Some(host) = url.host_str().map(str::to_ascii_lowercase) else {
        return Ok(None);
    };
    if let Some(excluded) = read(["NO_PROXY", "no_proxy"])
        && excluded.split(',').map(str::trim).any(|entry| {
            let entry = entry.trim_start_matches('.').to_ascii_lowercase();
            entry == "*"
                || (!entry.is_empty() && (host == entry || host.ends_with(&format!(".{entry}"))))
        })
    {
        return Ok(None);
    }
    parse_proxy(&proxy).map(Some)
}

/// A proxy value as curl reads it: a URL, or `host:port` with `http://`
/// assumed. Only an `http` or `https` proxy with a host is usable.
///
/// # Errors
///
/// [`SourceFetchReason::ProxyUnusable`] for anything else. A proxy the user
/// set is never ignored: going direct would bypass it.
fn parse_proxy(value: &str) -> Result<Url, SourceFetchReason> {
    let value = value.trim();
    let parsed = if value.contains("://") {
        Url::parse(value)
    } else {
        Url::parse(&format!("http://{value}"))
    };
    let proxy = parsed.map_err(|_| SourceFetchReason::ProxyUnusable)?;
    let usable = matches!(proxy.scheme(), "http" | "https")
        && proxy.host_str().is_some_and(|host| !host.is_empty());
    if usable {
        Ok(proxy)
    } else {
        Err(SourceFetchReason::ProxyUnusable)
    }
}

/// The receiving end of a fetch: the session's source slot. A fetch hands it
/// every chunk in order and reads back what it has.
pub(crate) trait LinkSink: Send {
    /// Takes the next chunk.
    ///
    /// # Errors
    ///
    /// A write failure, reported as the sink's own error.
    fn append(&mut self, chunk: &[u8]) -> Result<(), LinkSinkError>;
    /// The first [`MAGIC_BYTES`] bytes, once that many have arrived.
    fn header(&self) -> Option<&[u8]>;
    /// Whether the slot's filesystem has room for `bytes` more than the slot
    /// already holds.
    ///
    /// # Errors
    ///
    /// [`LinkSinkError::NoRoom`] when it was read and is short.
    fn ensure_room(&self, bytes: u64) -> Result<(), LinkSinkError>;
}

/// Why a sink refused.
#[derive(Debug)]
pub(crate) enum LinkSinkError {
    /// The filesystem has no room.
    NoRoom,
    /// Writing failed.
    Write(std::io::Error),
}

/// What a fetch reports when it fails.
#[derive(Debug)]
pub(crate) enum LinkFetchError {
    /// The online step failed for this reason.
    Refused(SourceFetchReason),
    /// The caller cancelled.
    Cancelled,
    /// The session's slot refused the bytes.
    Sink(LinkSinkError),
}

/// What a completed fetch says about where its bytes came from.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FetchedLink {
    pub(crate) link_sha256: Sha256Hex,
    pub(crate) requested_host: HostName,
    pub(crate) final_host: HostName,
    pub(crate) redirects: u8,
    pub(crate) address_check: AddressCheck,
    pub(crate) bytes: u64,
}

/// The route a fetch takes: production, or the development-only test route.
#[derive(Clone, Default)]
pub(crate) struct LinkRoute {
    #[cfg(any(test, feature = "install-test-hooks"))]
    test: Option<LinkTestRoute>,
}

impl LinkRoute {
    /// The production route.
    pub(crate) const fn production() -> Self {
        Self {
            #[cfg(any(test, feature = "install-test-hooks"))]
            test: None,
        }
    }

    /// The test route.
    #[cfg(any(test, feature = "install-test-hooks"))]
    pub(crate) fn test(route: LinkTestRoute) -> Self {
        Self { test: Some(route) }
    }

    fn stall(&self) -> Duration {
        #[cfg(any(test, feature = "install-test-hooks"))]
        if let Some(route) = &self.test {
            return route.stall;
        }
        STALL_DEADLINE
    }

    fn room_step(&self) -> u64 {
        #[cfg(any(test, feature = "install-test-hooks"))]
        if let Some(route) = &self.test {
            return route.room_step;
        }
        ROOM_STEP
    }

    /// The free space the test route simulates for the source slot, if any.
    #[cfg(any(test, feature = "install-test-hooks"))]
    pub(crate) fn free_space(&self) -> Option<u64> {
        self.test.as_ref().and_then(|route| route.free_space)
    }

    fn allowance(&self) -> Allowance {
        #[cfg(any(test, feature = "install-test-hooks"))]
        if let Some(route) = &self.test {
            return Allowance {
                test_port: Some(route.port),
                test_loopback: true,
            };
        }
        Allowance::default()
    }
}

/// Answers of a test resolver: each lookup of a name takes the next answer,
/// and the last answer repeats.
#[cfg(any(test, feature = "install-test-hooks"))]
#[derive(Debug, Default)]
pub struct TestAnswers {
    answers: std::sync::Mutex<
        std::collections::BTreeMap<String, std::collections::VecDeque<Vec<IpAddr>>>,
    >,
}

#[cfg(any(test, feature = "install-test-hooks"))]
impl TestAnswers {
    /// Answers `name` with each list in turn, the last one from then on.
    pub fn answer(&self, name: &str, answers: Vec<Vec<IpAddr>>) {
        if let Ok(mut table) = self.answers.lock() {
            table.insert(name.to_owned(), answers.into());
        }
    }

    fn next(&self, name: &str) -> Vec<IpAddr> {
        let Ok(mut table) = self.answers.lock() else {
            return Vec::new();
        };
        let Some(queue) = table.get_mut(name) else {
            return Vec::new();
        };
        if queue.len() > 1 {
            queue.pop_front().unwrap_or_default()
        } else {
            queue.front().cloned().unwrap_or_default()
        }
    }
}

/// A route to a local HTTPS test server: its port, the root certificate it
/// is signed by, the resolver's answers and an optional explicit proxy.
///
/// Development builds only (`install-test-hooks`, refused in a release build
/// and by the governance check), like the managed transfer's loopback route:
/// it trusts a test root and allows `127.0.0.1`, which no production fetch
/// does.
#[cfg(any(test, feature = "install-test-hooks"))]
#[derive(Clone, Debug)]
pub struct LinkTestRoute {
    port: u16,
    root_certificate_pem: Vec<u8>,
    answers: Arc<TestAnswers>,
    proxy: Option<String>,
    stall: Duration,
    room_step: u64,
    free_space: Option<u64>,
}

#[cfg(any(test, feature = "install-test-hooks"))]
impl LinkTestRoute {
    /// A route to a server on `127.0.0.1:port` whose certificate chains to
    /// `root_certificate_pem`, with names resolved by `answers`.
    #[must_use]
    pub const fn new(port: u16, root_certificate_pem: Vec<u8>, answers: Arc<TestAnswers>) -> Self {
        Self {
            port,
            root_certificate_pem,
            answers,
            proxy: None,
            stall: STALL_DEADLINE,
            room_step: ROOM_STEP,
            free_space: None,
        }
    }

    /// The same route with a shorter stall deadline, so a stall test does
    /// not wait 30 seconds.
    #[must_use]
    pub const fn with_stall(mut self, stall: Duration) -> Self {
        self.stall = stall;
        self
    }

    /// The same route through an explicit proxy, read as a proxy variable's
    /// value is (`http://127.0.0.1:<port>`, or `127.0.0.1:<port>`).
    #[must_use]
    pub fn through_proxy(mut self, proxy: &str) -> Self {
        self.proxy = Some(proxy.to_owned());
        self
    }

    /// The same route with the source slot's filesystem reporting
    /// `available` bytes free before the fetch (less what the slot has
    /// written since), judged by the production rule, so a short disk is
    /// tested without filling one.
    #[must_use]
    pub const fn with_free_space(mut self, available: u64) -> Self {
        self.free_space = Some(available);
        self
    }

    /// The same route checking room every `bytes` of a body of unknown
    /// length instead of every 64 MiB, so the check is seen with a small
    /// body.
    #[must_use]
    pub const fn with_room_step(mut self, bytes: u64) -> Self {
        self.room_step = bytes;
        self
    }
}

/// The client of one fetch.
fn client(
    route: &LinkRoute,
    allowance: Allowance,
    proxy: Option<&Url>,
    redirects: &Arc<AtomicU8>,
) -> Result<Client, SourceFetchReason> {
    let counter = Arc::clone(redirects);
    let proxied = proxy.is_some();
    let redirect = Policy::custom(move |attempt| {
        // `previous()` holds every URL already requested, the link included,
        // so it is 1 at the first redirect and 4 only at a fourth (reqwest's
        // own limit compares the same way).
        if attempt.previous().len() > usize::from(MAX_SOURCE_REDIRECTS) {
            return attempt.error(PolicyRefusal(SourceFetchReason::RedirectPolicy));
        }
        match check_link(attempt.url(), allowance) {
            Ok(()) if proxied && internal_through_proxy(attempt.url()) => {
                attempt.error(PolicyRefusal(SourceFetchReason::AddressRefused))
            }
            Ok(()) => {
                counter.fetch_add(1, Ordering::Relaxed);
                attempt.follow()
            }
            Err(SourceFetchReason::NotHttps) => {
                attempt.error(PolicyRefusal(SourceFetchReason::RedirectPolicy))
            }
            Err(reason) => attempt.error(PolicyRefusal(reason)),
        }
    });
    let builder = Client::builder()
        .tls_backend_native()
        .https_only(true)
        .tls_version_min(reqwest::tls::Version::TLS_1_2)
        .redirect(redirect)
        .referer(false)
        .no_gzip()
        .no_brotli()
        .no_zstd()
        .no_deflate()
        .connect_timeout(CONNECT_DEADLINE)
        .read_timeout(route.stall())
        .user_agent(user_agent())
        .no_proxy();
    let resolver = PublicAddressResolver {
        allowance,
        #[cfg(any(test, feature = "install-test-hooks"))]
        answers: route.test.as_ref().map(|test| Arc::clone(&test.answers)),
    };
    let mut builder = builder.dns_resolver(resolver);
    if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::https(proxy.clone()).map_err(|_| SourceFetchReason::ProxyUnusable)?,
        );
    }
    #[cfg(any(test, feature = "install-test-hooks"))]
    if let Some(test) = &route.test {
        let root = reqwest::Certificate::from_pem(&test.root_certificate_pem)
            .map_err(|_| SourceFetchReason::Tls)?;
        builder = builder.add_root_certificate(root);
    }
    #[cfg(not(any(test, feature = "install-test-hooks")))]
    let _ = route;
    builder.build().map_err(|_| SourceFetchReason::Tls)
}

/// The proxy a fetch uses: the test route's, or the environment's, each read
/// by [`parse_proxy`].
///
/// # Errors
///
/// [`SourceFetchReason::ProxyUnusable`] for a proxy that is named but
/// unusable.
fn proxy_for(route: &LinkRoute, url: &Url) -> Result<Option<Url>, SourceFetchReason> {
    #[cfg(any(test, feature = "install-test-hooks"))]
    if let Some(test) = &route.test {
        return test.proxy.as_deref().map(parse_proxy).transpose();
    }
    #[cfg(not(any(test, feature = "install-test-hooks")))]
    let _ = route;
    environment_proxy(url)
}

/// Fetches `link` into `sink`, within `limits`, checking `cancellation` at
/// least every 200 ms while it waits for the response and for each chunk
/// (not while a chunk is written: see the module documentation).
///
/// # Errors
///
/// [`LinkFetchError::Refused`] with the typed reason, [`LinkFetchError::Cancelled`],
/// or the sink's own error. The sink may hold a partial body on any error;
/// its owner discards it.
#[allow(
    clippy::too_many_lines,
    reason = "one auditable path from the link's checks to the last byte"
)]
pub(crate) async fn fetch_link(
    link: &SourceLink,
    limits: DownloadLimits,
    route: &LinkRoute,
    sink: &mut (dyn LinkSink + Send),
    cancellation: &dyn StageCancellation,
) -> Result<FetchedLink, LinkFetchError> {
    let refused = LinkFetchError::Refused;
    let url = Url::parse(link.expose()).map_err(|_| refused(SourceFetchReason::NotHttps))?;
    let allowance = route.allowance();
    check_link(&url, allowance).map_err(refused)?;
    let requested_host = host_name(&url).ok_or(refused(SourceFetchReason::NotHttps))?;
    let proxy = proxy_for(route, &url).map_err(refused)?;
    let address_check = if proxy.is_some() {
        if internal_through_proxy(&url) {
            return Err(refused(SourceFetchReason::AddressRefused));
        }
        AddressCheck::ProxyResolved
    } else {
        AddressCheck::Direct
    };
    let redirects = Arc::new(AtomicU8::new(0));
    let client = client(route, allowance, proxy.as_ref(), &redirects).map_err(refused)?;
    let deadline = Instant::now() + Duration::from_secs(u64::from(limits.max_seconds()));
    let request = client
        .get(url.clone())
        .header(ACCEPT, "*/*")
        .header(ACCEPT_ENCODING, "identity")
        .send();
    let mut response = wait(deadline, cancellation, request)
        .await?
        .map_err(|error| refused(classify(&error)))?;
    let declared = check_response(&response, limits).map_err(refused)?;
    // Room for the declared length, or for the first block of a body of
    // unknown length; such a body asks again as each later block begins.
    let room_step = route.room_step().max(1);
    let mut room_through = declared.unwrap_or_else(|| room_step.min(limits.max_bytes()));
    sink.ensure_room(room_through)
        .map_err(LinkFetchError::Sink)?;
    let mut received = 0_u64;
    let mut judged = false;
    loop {
        let chunk = wait(deadline, cancellation, response.chunk())
            .await?
            .map_err(|error| {
                if error.is_timeout() {
                    refused(SourceFetchReason::Stalled)
                } else if declared.is_some() && is_body_cut(&error) {
                    refused(SourceFetchReason::Truncated)
                } else {
                    refused(classify(&error))
                }
            })?;
        let Some(chunk) = chunk else {
            break;
        };
        let after = received
            .checked_add(u64::try_from(chunk.len()).map_err(|_| refused(SourceFetchReason::Size))?)
            .ok_or(refused(SourceFetchReason::Size))?;
        if after > limits.max_bytes() {
            return Err(refused(SourceFetchReason::Size));
        }
        if after > room_through {
            // The next block: room from what the slot holds now to its end.
            let through = room_through
                .saturating_add(room_step)
                .max(after)
                .min(limits.max_bytes());
            sink.ensure_room(through - received)
                .map_err(LinkFetchError::Sink)?;
            room_through = through;
        }
        sink.append(&chunk).map_err(LinkFetchError::Sink)?;
        received = after;
        if !judged && let Some(header) = sink.header() {
            judge_header(header).map_err(refused)?;
            judged = true;
        }
    }
    if !judged {
        // Fewer than twelve bytes: no admitted container is that small.
        return Err(refused(SourceFetchReason::NotAVideo));
    }
    if declared.is_some_and(|declared| declared != received) {
        return Err(refused(SourceFetchReason::Truncated));
    }
    let final_host = host_name(response.url()).ok_or(refused(SourceFetchReason::RedirectPolicy))?;
    let mut digest = Sha256::new();
    digest.update(url.as_str().as_bytes());
    let link_sha256 = Sha256Hex::parse(hex(&digest.finalize()))
        .map_err(|_| refused(SourceFetchReason::NotHttps))?;
    Ok(FetchedLink {
        link_sha256,
        requested_host,
        final_host,
        redirects: redirects.load(Ordering::Relaxed),
        address_check,
        bytes: received,
    })
}

/// Waits for `future` until `deadline`, looking at `cancellation` every
/// [`CANCEL_POLL`].
async fn wait<F: Future>(
    deadline: Instant,
    cancellation: &dyn StageCancellation,
    future: F,
) -> Result<F::Output, LinkFetchError> {
    let mut future = std::pin::pin!(future);
    loop {
        if cancellation.is_cancelled() {
            return Err(LinkFetchError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(LinkFetchError::Refused(SourceFetchReason::Deadline));
        }
        let slice = (Instant::now() + CANCEL_POLL).min(deadline);
        if let Ok(output) = timeout_at(slice, future.as_mut()).await {
            return Ok(output);
        }
    }
}

/// Accepts one complete `200 OK` identity body that is not a web page and
/// declares no more than the limit; returns the declared length.
fn check_response(
    response: &reqwest::Response,
    limits: DownloadLimits,
) -> Result<Option<u64>, SourceFetchReason> {
    let status = response.status();
    if status == StatusCode::PROXY_AUTHENTICATION_REQUIRED {
        return Err(SourceFetchReason::ProxyAuth);
    }
    if status == StatusCode::PARTIAL_CONTENT || response.headers().contains_key(CONTENT_RANGE) {
        return Err(SourceFetchReason::Partial);
    }
    if status != StatusCode::OK {
        return Err(SourceFetchReason::HttpStatus);
    }
    if response
        .headers()
        .get(CONTENT_ENCODING)
        .is_some_and(|encoding| !encoding.as_bytes().eq_ignore_ascii_case(b"identity"))
    {
        return Err(SourceFetchReason::Compressed);
    }
    if response.headers().get(CONTENT_TYPE).is_some_and(|value| {
        let value = value.as_bytes().to_ascii_lowercase();
        value.starts_with(b"text/") || value.starts_with(b"application/xhtml")
    }) {
        return Err(SourceFetchReason::WebPage);
    }
    let declared = response.content_length();
    if declared.is_some_and(|bytes| bytes > limits.max_bytes()) {
        return Err(SourceFetchReason::Size);
    }
    Ok(declared)
}

/// Judges the first twelve bytes: an admitted container, a web page, or
/// neither.
fn judge_header(header: &[u8]) -> Result<(), SourceFetchReason> {
    let iso = header.get(4..8) == Some(b"ftyp".as_slice());
    let matroska = header.get(..4) == Some([0x1a, 0x45, 0xdf, 0xa3].as_slice());
    if iso || matroska {
        return Ok(());
    }
    let text = header
        .iter()
        .skip_while(|byte| {
            byte.is_ascii_whitespace() || **byte == 0xef || **byte == 0xbb || **byte == 0xbf
        })
        .copied()
        .collect::<Vec<u8>>();
    if text.first() == Some(&b'<') {
        Err(SourceFetchReason::WebPage)
    } else {
        Err(SourceFetchReason::NotAVideo)
    }
}

/// The host of `url` in the form a record keeps.
fn host_name(url: &Url) -> Option<HostName> {
    HostName::parse(&url.host_str()?.to_ascii_lowercase()).ok()
}

/// Whether a body error is the connection ending before the declared length.
fn is_body_cut(error: &reqwest::Error) -> bool {
    error.is_body() || error.is_decode()
}

/// The typed reason of a transport failure. Only the error's kind and the
/// types in its chain are read, never its text, except the one
/// proxy-authentication marker ([`PROXY_AUTH_REQUIRED`]).
fn classify(error: &reqwest::Error) -> SourceFetchReason {
    let mut source: Option<&(dyn Error + 'static)> = Some(error);
    while let Some(current) = source {
        if let Some(PolicyRefusal(reason)) = current.downcast_ref::<PolicyRefusal>() {
            return *reason;
        }
        if current.is::<native_tls::Error>() {
            return SourceFetchReason::Tls;
        }
        if current.source().is_none() && current.to_string().ends_with(PROXY_AUTH_REQUIRED) {
            return SourceFetchReason::ProxyAuth;
        }
        source = current.source();
    }
    if error.is_redirect() {
        SourceFetchReason::RedirectPolicy
    } else if error.is_timeout() {
        SourceFetchReason::Stalled
    } else {
        SourceFetchReason::Offline
    }
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(char::from(DIGITS[usize::from(byte >> 4)]));
        text.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(text: &str) -> Result<IpAddr, std::net::AddrParseError> {
        text.parse()
    }

    /// Production policy, with no hook: loopback, private, link-local,
    /// metadata and the other special ranges are refused, and public
    /// addresses are not.
    #[test]
    fn only_public_unicast_addresses_are_reachable() -> Result<(), Box<dyn Error>> {
        for refused in [
            "0.0.0.0",
            "10.1.2.3",
            "100.64.0.1",
            "100.100.100.200",
            "127.0.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "172.31.255.255",
            "192.0.0.8",
            "192.0.2.1",
            "192.88.99.1",
            "192.168.1.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "::ffff:8.8.8.8",
            "::8.8.8.8",
            "64:ff9b::7f00:1",
            "64:ff9b:1::1",
            "100::1",
            "2001::1",
            "2001:db8::1",
            "2001:2::1",
            "2001:10::1",
            "2002:7f00:1::1",
            "2002:c0a8:101::1",
            "3fff::1",
            "fc00::1",
            "fd00:ec2::254",
            "fe80::1",
            "fec0::1",
            "ff02::1",
        ] {
            assert!(!is_public_address(ip(refused)?), "{refused}");
        }
        for allowed in [
            "8.8.8.8",
            "1.1.1.1",
            "100.63.255.255",
            "100.128.0.1",
            "172.32.0.1",
            "198.20.0.1",
            "2606:4700::1111",
            "2a00:1450:4001::1",
            "2002:808:808::1",
            "64:ff9b::808:808",
        ] {
            assert!(is_public_address(ip(allowed)?), "{allowed}");
        }
        Ok(())
    }

    #[test]
    fn links_must_be_https_on_443_without_credentials() -> Result<(), Box<dyn Error>> {
        let production = Allowance::default();
        for (link, reason) in [
            (
                "http://videos.example.org/a.mp4",
                SourceFetchReason::NotHttps,
            ),
            (
                "https://user:pass@videos.example.org/a.mp4",
                SourceFetchReason::CredentialsInLink,
            ),
            (
                "https://user@videos.example.org/a.mp4",
                SourceFetchReason::CredentialsInLink,
            ),
            (
                "https://videos.example.org:8443/a.mp4",
                SourceFetchReason::PortNotAllowed,
            ),
            ("https://127.0.0.1/a.mp4", SourceFetchReason::AddressRefused),
            ("https://[::1]/a.mp4", SourceFetchReason::AddressRefused),
            (
                "https://169.254.169.254/latest/meta-data",
                SourceFetchReason::AddressRefused,
            ),
            (
                "https://[::ffff:10.0.0.1]/a.mp4",
                SourceFetchReason::AddressRefused,
            ),
        ] {
            assert_eq!(
                check_link(&Url::parse(link)?, production),
                Err(reason),
                "{link}"
            );
        }
        for link in [
            "https://videos.example.org/a.mp4?token=x#t=1",
            "https://videos.example.org:443/a.mp4",
            "https://8.8.8.8/a.mp4",
        ] {
            assert_eq!(check_link(&Url::parse(link)?, production), Ok(()), "{link}");
        }
        Ok(())
    }

    #[test]
    fn internal_names_are_recognised() {
        for name in [
            "localhost",
            "intranet",
            "files.localhost",
            "nas.local",
            "db.internal",
            "printer.lan",
            "router.home.arpa",
            "wiki.corp",
            "portal.intranet",
        ] {
            assert!(is_internal_name(name), "{name}");
        }
        for name in [
            "videos.example.org",
            "images-assets.nasa.gov",
            "archive.org",
        ] {
            assert!(!is_internal_name(name), "{name}");
        }
    }

    #[test]
    fn a_proxy_value_is_a_url_or_host_and_port_and_anything_else_is_refused()
    -> Result<(), Box<dyn Error>> {
        for (value, proxy) in [
            ("proxy.corp:3128", "http://proxy.corp:3128/"),
            ("127.0.0.1:3128", "http://127.0.0.1:3128/"),
            (
                " http://proxy.example.org:8080 ",
                "http://proxy.example.org:8080/",
            ),
            ("https://proxy.example.org", "https://proxy.example.org/"),
            (
                "http://user:secret@proxy.example.org:3128",
                "http://user:secret@proxy.example.org:3128/",
            ),
        ] {
            assert_eq!(parse_proxy(value)?.as_str(), proxy, "{value}");
        }
        for value in [
            "http://bad host:3128",
            "socks5://proxy.example.org:1080",
            "ftp://proxy.example.org",
            "http://",
            "proxy.example.org:port",
            "file:///proxy",
        ] {
            assert_eq!(
                parse_proxy(value),
                Err(SourceFetchReason::ProxyUnusable),
                "{value}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_set_proxy_variable_is_used_or_refused_never_ignored() -> Result<(), Box<dyn Error>> {
        const UNUSABLE: &str = "http://bad host:3128";
        let link = Url::parse("https://videos.example.org/a.mp4")?;
        let proxy = |pairs: &[(&str, &str)]| {
            proxy_from_variables(&link, |name| {
                pairs
                    .iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| (*value).to_owned())
            })
            .map(|found| found.map(String::from))
        };
        assert_eq!(proxy(&[]), Ok(None));
        assert_eq!(
            proxy(&[("https_proxy", "proxy.corp:3128")]),
            Ok(Some("http://proxy.corp:3128/".to_owned()))
        );
        assert_eq!(
            proxy(&[("ALL_PROXY", "http://all.example.org:8080")]),
            Ok(Some("http://all.example.org:8080/".to_owned()))
        );
        // Unusable: refused, whichever variable names it.
        for name in ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] {
            assert_eq!(
                proxy(&[(name, UNUSABLE)]),
                Err(SourceFetchReason::ProxyUnusable),
                "{name}"
            );
        }
        // A host NO_PROXY excludes goes direct, whatever the proxy value.
        for (name, excluded) in [
            ("NO_PROXY", "videos.example.org"),
            ("no_proxy", "example.org"),
            ("NO_PROXY", ".example.org"),
            ("NO_PROXY", "*"),
            ("NO_PROXY", "other.test, example.org"),
        ] {
            assert_eq!(
                proxy(&[("HTTPS_PROXY", UNUSABLE), (name, excluded)]),
                Ok(None),
                "{excluded}"
            );
        }
        // A suffix matches only at a label boundary.
        assert_eq!(
            proxy(&[("HTTPS_PROXY", UNUSABLE), ("NO_PROXY", "ample.org")]),
            Err(SourceFetchReason::ProxyUnusable)
        );
        Ok(())
    }

    #[test]
    fn through_a_proxy_internal_names_are_refused_and_addresses_left_to_the_link_check()
    -> Result<(), Box<dyn Error>> {
        for link in [
            "https://wiki.corp/x.mp4",
            "https://intranet/x.mp4",
            "https://nas.local./x.mp4",
        ] {
            assert!(internal_through_proxy(&Url::parse(link)?), "{link}");
        }
        for link in ["https://videos.example.org/x.mp4", "https://8.8.8.8/x.mp4"] {
            assert!(!internal_through_proxy(&Url::parse(link)?), "{link}");
        }
        Ok(())
    }

    #[test]
    fn a_name_with_any_private_address_is_refused_whole() -> Result<(), Box<dyn Error>> {
        let production = Allowance::default();
        assert!(screen(vec![ip("8.8.8.8")?], production).is_ok());
        let mixed = screen(vec![ip("8.8.8.8")?, ip("10.0.0.1")?], production);
        assert!(matches!(
            mixed.err().and_then(|error| error.downcast::<PolicyRefusal>().ok()),
            Some(refusal) if refusal.0 == SourceFetchReason::AddressRefused
        ));
        assert!(screen(Vec::new(), production).is_err());
        Ok(())
    }

    #[test]
    fn the_first_bytes_decide_the_container() {
        assert_eq!(judge_header(b"\0\0\0\x18ftypisom"), Ok(()));
        assert_eq!(
            judge_header(&[0x1a, 0x45, 0xdf, 0xa3, 0, 0, 0, 0, 0, 0, 0, 0]),
            Ok(())
        );
        assert_eq!(
            judge_header(b"<!DOCTYPE ht"),
            Err(SourceFetchReason::WebPage)
        );
        assert_eq!(
            judge_header(b"  \n<html lan"),
            Err(SourceFetchReason::WebPage)
        );
        assert_eq!(
            judge_header(b"#EXTM3U\n#EXT"),
            Err(SourceFetchReason::NotAVideo)
        );
        assert_eq!(
            judge_header(b"RIFF\0\0\0\0AVI "),
            Err(SourceFetchReason::NotAVideo)
        );
    }

    #[test]
    fn the_user_agent_names_only_the_product() {
        let agent = user_agent();
        assert!(agent.starts_with("VSift/"), "{agent}");
        assert!(agent.ends_with(" link-ingest"), "{agent}");
        assert!(!agent.contains('@'), "{agent}");
    }
}
