//! The skill's two budget profiles, as the handoff writes their limits.
//!
//! `skills/vsift/references/budgets.md` owns the table; these constants are
//! its copy for the rule that given `budget.limits` must be the profile's
//! own. The CLI's `skill_contract` guard and the trial grader's policy tests
//! fail when the table and these values differ.

/// Every limit of one budget profile, in the handoff's units (bytes and
/// seconds).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HandoffBudgetLimits {
    /// Images opened before the next reasoning step.
    pub images_per_step: u64,
    /// Images opened in total, the image check included.
    pub images_total: u64,
    /// Bytes of opened images.
    pub image_bytes: u64,
    /// The `--limit` of paged commands.
    pub page_limit: u64,
    /// Commands and image opens.
    pub tool_calls: u64,
    /// Refinements in a row for one claim.
    pub refinement_depth: u64,
    /// Seconds from the first command.
    pub wall_time_s: u64,
    /// The `--max-frames` of one burst.
    pub burst_frames: u64,
}

impl HandoffBudgetLimits {
    /// Each limit with its handoff member name, in the table's order.
    #[must_use]
    pub const fn members(&self) -> [(&'static str, u64); 8] {
        [
            ("images_per_step", self.images_per_step),
            ("images_total", self.images_total),
            ("image_bytes", self.image_bytes),
            ("page_limit", self.page_limit),
            ("tool_calls", self.tool_calls),
            ("refinement_depth", self.refinement_depth),
            ("wall_time_s", self.wall_time_s),
            ("burst_frames", self.burst_frames),
        ]
    }
}

/// The `compact` profile: the default, for small models and one-image
/// clients.
pub const COMPACT_BUDGET: HandoffBudgetLimits = HandoffBudgetLimits {
    images_per_step: 1,
    images_total: 6,
    image_bytes: 12 * 1024 * 1024,
    page_limit: 20,
    tool_calls: 30,
    refinement_depth: 2,
    wall_time_s: 15 * 60,
    burst_frames: 4,
};

/// The `standard` profile.
pub const STANDARD_BUDGET: HandoffBudgetLimits = HandoffBudgetLimits {
    images_per_step: 4,
    images_total: 24,
    image_bytes: 48 * 1024 * 1024,
    page_limit: 50,
    tool_calls: 80,
    refinement_depth: 4,
    wall_time_s: 30 * 60,
    burst_frames: 12,
};

/// The limits of a profile named in a handoff (`compact` or `standard`).
#[must_use]
pub fn budget_profile(name: &str) -> Option<HandoffBudgetLimits> {
    match name {
        "compact" => Some(COMPACT_BUDGET),
        "standard" => Some(STANDARD_BUDGET),
        _ => None,
    }
}
