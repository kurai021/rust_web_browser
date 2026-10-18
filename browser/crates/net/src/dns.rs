//! Optional DNS-over-HTTPS adapter. Hickory implements DNS; no query runs at startup.
use hickory_resolver::{
    config::{ResolverConfig, QUAD9},
    net::runtime::TokioRuntimeProvider,
    TokioResolver,
};
pub(crate) struct Doh {
    resolver: std::sync::Arc<std::sync::OnceLock<Result<TokioResolver, String>>>,
}
impl Doh {
    pub(crate) fn quad9() -> Self {
        Self {
            resolver: Default::default(),
        }
    }
}
impl reqwest::dns::Resolve for Doh {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let resolver = self.resolver.clone();
        let hostname = name.as_str().to_owned();
        Box::pin(async move {
            let resolver = resolver.get_or_init(|| {
                let mut builder = TokioResolver::builder_with_config(
                    ResolverConfig::https(&QUAD9),
                    TokioRuntimeProvider::default(),
                );
                builder.options_mut().timeout = std::time::Duration::from_secs(5);
                builder.options_mut().attempts = 2;
                builder.options_mut().cache_size = 256;
                builder.build().map_err(|e| e.to_string())
            });
            let resolver = resolver
                .as_ref()
                .map_err(|message| std::io::Error::other(message.clone()))?;
            let lookup = resolver.lookup_ip(hostname).await?;
            let addresses = lookup
                .iter()
                .map(|ip| std::net::SocketAddr::new(ip, 0))
                .collect::<Vec<_>>();
            Ok(Box::new(addresses.into_iter()) as reqwest::dns::Addrs)
        })
    }
}
