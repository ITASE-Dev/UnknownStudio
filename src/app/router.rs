/// Stable hash of a project's on-disk folder name. Numeric so `AppRoute`
/// stays `Copy`; resolve it through `ProjectLibrary::find`.
pub type ProjectId = u64;

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum AppRoute {
    #[default]
    Dashboard,
    Onboarding,
    Studio(ProjectId),
    Growth(ProjectId),
}

impl AppRoute {
    pub fn title(self) -> &'static str {
        match self {
            Self::Dashboard => "Projects",
            Self::Onboarding => "New Project",
            Self::Studio(_) => "Studio",
            Self::Growth(_) => "Growth & Export",
        }
    }

    pub fn project(self) -> Option<ProjectId> {
        match self {
            Self::Studio(id) | Self::Growth(id) => Some(id),
            _ => None,
        }
    }
}

/// Tracks route changes each frame so menus can offer a Back action without the
/// views needing to know anything beyond `&mut AppRoute`.
#[derive(Default)]
pub struct RouteHistory {
    last: Option<AppRoute>,
    stack: Vec<AppRoute>,
}

impl RouteHistory {
    /// Call once per frame, after the views have had a chance to navigate.
    pub fn track(&mut self, current: AppRoute) {
        if self.last != Some(current) {
            if let Some(prev) = self.last {
                self.stack.push(prev);
                if self.stack.len() > 32 {
                    self.stack.remove(0);
                }
            }
            self.last = Some(current);
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.stack.is_empty()
    }

    pub fn back(&mut self, current: &mut AppRoute) {
        if let Some(prev) = self.stack.pop() {
            *current = prev;
            self.last = Some(prev);
        }
    }
}
