use crate::presentation::brightness::UserFacingError;
use crate::presentation::settings::SettingsAction;
use crate::settings_view::SettingsIntent;
use crate::updates::UpdateChannel;
use crate::upgrade_preflight::InstallationOwnership;
use crate::version::VersionInfo;

/// Update availability on the checked channel, without an installation target.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckReport {
    pub installed_version: String,
    pub channel: UpdateChannel,
    pub update_available: bool,
    pub warning: Option<String>,
    pub installation_ownership: InstallationOwnership,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCheckPresentation {
    installed_version_label: String,
    checking: bool,
    install_active: bool,
    result: Option<UpdateCheckReport>,
    installation_ownership: Option<InstallationOwnership>,
    error: Option<UserFacingError>,
}

impl Default for UpdateCheckPresentation {
    fn default() -> Self {
        let version = VersionInfo::current();
        Self {
            installed_version_label: format!(
                "{} ({})",
                version.version(),
                version.channel().as_str()
            ),
            checking: false,
            install_active: false,
            result: None,
            installation_ownership: None,
            error: None,
        }
    }
}

impl UpdateCheckPresentation {
    pub fn installed_version_label(&self) -> &str {
        &self.installed_version_label
    }

    pub fn checking(&self) -> bool {
        self.checking
    }

    pub fn installation_ownership(&self) -> Option<InstallationOwnership> {
        self.installation_ownership.or_else(|| {
            self.result
                .as_ref()
                .map(|report| report.installation_ownership)
        })
    }

    pub fn check_action(&self) -> SettingsAction {
        SettingsAction::new(
            if self.checking {
                "Checking…"
            } else if self.error.is_some() {
                "Retry check"
            } else {
                "Check for updates"
            },
            !self.checking
                && !self.install_active
                && !self
                    .installation_ownership()
                    .is_some_and(InstallationOwnership::externally_updated),
            SettingsIntent::CheckForUpdates,
        )
    }

    pub fn result(&self) -> Option<&UpdateCheckReport> {
        self.result.as_ref()
    }

    pub fn error(&self) -> Option<&UserFacingError> {
        self.error.as_ref()
    }

    pub(crate) fn set_install_active(&mut self, active: bool) {
        self.install_active = active;
    }

    pub(crate) fn start(&mut self) {
        self.checking = true;
        self.error = None;
    }

    pub(crate) fn clear_result(&mut self) {
        self.result = None;
        self.error = None;
    }

    pub(crate) fn set_installation_ownership(&mut self, ownership: InstallationOwnership) {
        self.installation_ownership = Some(ownership);
        if let Some(report) = &mut self.result {
            report.installation_ownership = ownership;
        }
    }

    pub(crate) fn complete(&mut self, result: Result<UpdateCheckReport, UserFacingError>) {
        self.checking = false;
        match result {
            Ok(mut report) => {
                if let Some(owner) = self
                    .installation_ownership
                    .filter(|owner| owner.externally_updated())
                {
                    report.installation_ownership = owner;
                }
                self.installation_ownership = Some(report.installation_ownership);
                self.result = Some(report);
                self.error = None;
            }
            Err(error) => self.error = Some(error),
        }
    }
}
