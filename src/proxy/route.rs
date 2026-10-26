//! A route and its vault credentials are frozen for the lifetime of a transfer.
use super::*;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub struct Route {
    pub(crate) settings: Settings,
    pub(crate) login: Option<Arc<Login>>,
}
impl Route {
    pub(crate) fn require_public_pinning(&self) -> Result<()> {
        ensure!(
            self.settings.mode == Mode::Direct && self.login.is_none(),
            "Site readers and their downloads require Direct connection in Preferences → Connection → Proxy; proxy routes cannot enforce public-only destination checks"
        );
        Ok(())
    }
    pub fn direct() -> Self {
        Self {
            settings: Settings {
                mode: Mode::Direct,
                ..Default::default()
            },
            login: None,
        }
    }
    pub fn load(store: &Store) -> Result<Self> {
        let _guard = guard()?;
        let settings = store.proxy_settings()?;
        let login = if let Some(id) = settings.credential {
            let bytes = vault::read(id)?.context(
                "Saved proxy login is unavailable. Re-enter it in Preferences → Connection → Proxy.",
            )?;
            let stored: StoredLogin = serde_json::from_slice(&bytes).map_err(|_| {
                anyhow::anyhow!("Saved proxy login is invalid. Re-enter it in Preferences.")
            })?;
            ensure!(
                stored.endpoint == settings.endpoint,
                "Saved proxy login belongs to a different proxy"
            );
            stored.login.validate(&endpoint(&settings.endpoint)?)?;
            Some(Arc::new(stored.login))
        } else {
            None
        };
        Ok(Self { settings, login })
    }
    pub(crate) fn enterprise(&self) -> bool {
        self.login.is_some() && self.settings.authentication != Authentication::Basic
    }
    pub(crate) fn configure(&self, builder: ClientBuilder) -> Result<ClientBuilder> {
        match self.settings.mode {
            Mode::System => Ok(builder),
            Mode::Direct => Ok(builder.no_proxy()),
            Mode::Custom if self.enterprise() => Ok(builder.no_proxy()), // execute() owns this route.
            Mode::Custom => {
                let mut proxy = reqwest::Proxy::all(endpoint(&self.settings.endpoint)?)
                    .map_err(|_| anyhow::anyhow!("Invalid proxy configuration"))?;
                if let Some(login) = &self.login {
                    proxy = proxy.basic_auth(&login.username, &login.password);
                }
                Ok(builder.no_proxy().proxy(proxy))
            }
        }
    }
    pub(crate) async fn execute(
        &self,
        client: &reqwest::Client,
        request: reqwest::Request,
        server: Option<Arc<crate::auth::Credentials>>,
    ) -> Result<reqwest::Response> {
        if self.enterprise() || server.is_some() {
            #[cfg(windows)]
            {
                crate::windows_http::send(self.clone(), request, server).await
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("NTLM and Negotiate authentication require Windows")
            }
        } else {
            crate::engine::network_wait(client.execute(request)).await
        }
    }
}
