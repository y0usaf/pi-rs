//! Spatiotemporal lifecycle for the TUI render substrate.
//!
//! Bridges pi's render-side components ([`crate::component::Component`]) and
//! terminal resources to the `pi-rs-kernel` spatiotemporal context so mounting
//! and unmounting follow the **temporal** axis (effects commit, inverses undo,
//! unmount replays them in reverse with no residue) and declared dependencies
//! follow the **spatial** axis (a committed `set` on a key notifies only the
//! declared readers, never a full repaint).
//!
//! This is the *lifecycle* seam only: rendered cells are produced by the same
//! render-side components as before, so replacing repaint-everything with
//! declared-reader invalidation never changes the observable frame.

use pi_rs_kernel::{Component as KernelComponent, Context, Inverse};
use std::any::Any;
use std::sync::Arc;

/// Dependency keys TUI components may declare as reads.
pub const KEY_THEME: &str = "theme";
pub const KEY_SETTINGS: &str = "settings";
pub const KEY_MODEL: &str = "model";
/// Host-owned resource key for editor state commits (scoped by inverse).
pub const KEY_EDITOR: &str = "pi-rs-tui:editor";

/// Identity of one dependency generation. A consumer keyed by this reloads
/// only when `[key, provider id, version, schema hash]` changes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Epoch {
    pub key: &'static str,
    pub provider_id: String,
    pub version: u64,
    pub schema_hash: u64,
}

impl Epoch {
    pub fn new(
        key: &'static str,
        provider_id: impl Into<String>,
        version: u64,
        schema_hash: u64,
    ) -> Self {
        Self {
            key,
            provider_id: provider_id.into(),
            version,
            schema_hash,
        }
    }

    /// A consumer at `self` must reload when the provider publishes `next`.
    /// Only an *epoch change* forces the reload; an equal identity leaves the
    /// mounted generation untouched.
    pub fn is_stale_against(&self, next: &Epoch) -> bool {
        self != next
    }
}

/// A render-side component bound to the kernel lifecycle: it declares the keys
/// it reads (`reads`), the effects it commits on mount (each returning an
/// inverse), and an optional reaction to a change on one of its declared keys.
pub struct ScopedComponent {
    kernel: KernelComponent,
    render: Option<Arc<dyn crate::component::Component>>,
}

impl ScopedComponent {
    /// Bind a render-side component to the kernel context, declaring the
    /// dependency keys it reads.
    pub fn new(
        reads: Vec<&'static str>,
        render: Arc<dyn crate::component::Component>,
    ) -> Self {
        Self {
            kernel: KernelComponent::new(reads),
            render: Some(render),
        }
    }

    /// Add an effect. `apply` receives the context and must return an inverse
    /// that restores what it committed. Effects run at mount; unmount replays
    /// their inverses in reverse order.
    pub fn effect(mut self, apply: impl Fn(&mut Context) -> Inverse + Send + 'static) -> Self {
        self.kernel = self.kernel.effect(apply);
        self
    }

    /// Declare a reaction to a change on one of this component's read keys.
    pub fn on_change(
        mut self,
        cb: impl FnMut(&mut Context, &'static str) + Send + 'static,
    ) -> Self {
        self.kernel = self.kernel.on_change(cb);
        self
    }

    /// Scope the module-wide terminal-image resources by inverse: snapshot at
    /// mount, restore at unmount, so the resource leaves no residue.
    pub fn terminal_image_scope(self) -> Self {
        self.effect(|_ctx| {
            let snapshot = crate::terminal_image::snapshot_image_state();
            Box::new(move |_ctx| crate::terminal_image::restore_image_state(snapshot))
        })
    }

    /// Scope editor state: commit `value` under [`KEY_EDITOR`] and restore the
    /// prior value (or remove it) on unmount.
    pub fn editor_state(self, value: impl Into<String> + Send + 'static) -> Self {
        let value: String = value.into();
        self.effect(move |ctx| {
            let prior = ctx.get::<String>(KEY_EDITOR).cloned();
            ctx.set(KEY_EDITOR, value.clone());
            Box::new(move |ctx| match prior {
                Some(prev) => ctx.set(KEY_EDITOR, prev),
                None => ctx.remove(KEY_EDITOR),
            })
        })
    }

    /// Consume into the kernel component for mounting.
    pub fn into_kernel(self) -> KernelComponent {
        self.kernel
    }
}

/// Register a subscription (an input watcher): a component that declares it
/// reads `key` and reacts when it changes. Unmounting the returned component's
/// scope cancels the subscription.
pub fn subscribe(
    key: &'static str,
    on_change: impl FnMut(&mut Context, &'static str) + Send + 'static,
) -> KernelComponent {
    KernelComponent::new(vec![key]).on_change(on_change)
}

/// Subscribe to `key` with generation tracking. On a committed change the
/// component reloads only when the published [`Epoch`] for its key differs from
/// the generation it last loaded; an equal identity does nothing.
pub fn subscribe_epoch(
    key: &'static str,
    mut current: Epoch,
    mut reload: impl FnMut(&mut Context, &'static str) + Send + 'static,
) -> KernelComponent {
    KernelComponent::new(vec![key]).on_change(move |ctx, changed_key| {
        if changed_key != key {
            return;
        }
        let Some(next) = ctx.get::<Epoch>(key) else {
            return;
        };
        if current.is_stale_against(next) {
            current = next.clone();
            reload(ctx, key);
        }
    })
}

/// Host-owned mount point tying the kernel context to the render-side tree.
pub struct TuiHost {
    ctx: Context,
    /// Mounted render-side components, in mount order, for deterministic frames.
    renderers: Vec<(usize, Arc<dyn crate::component::Component>)>,
}

impl Default for TuiHost {
    fn default() -> Self {
        Self::new()
    }
}

impl TuiHost {
    pub fn new() -> Self {
        Self {
            ctx: Context::new(),
            renderers: Vec::new(),
        }
    }

    /// Mount a component: apply its effects, register its reads, and attach its
    /// render side. Returns the scope id for later `unmount`.
    pub fn mount(&mut self, component: ScopedComponent) -> usize {
        let render = component.render;
        let id = self.ctx.mount(component.kernel);
        if let Some(render) = render {
            self.renderers.push((id, render));
        }
        id
    }

    /// Mount a bare kernel component (subscription, watcher, resource scope).
    pub fn mount_kernel(&mut self, component: KernelComponent) -> usize {
        self.ctx.mount(component)
    }

    /// Unmount a scope: replay every inverse in reverse and unregister reads.
    /// Render side is detached and the context returns to its pre-mount state.
    pub fn unmount(&mut self, id: usize) {
        self.ctx.unmount(id);
        self.renderers.retain(|(scope, _)| *scope != id);
    }

    /// The single committed write path on a declared key. Notifies only the
    /// components that declared `key` as a read.
    pub fn set<T: Any + Send>(&mut self, key: &'static str, value: T) {
        self.ctx.set(key, value);
    }

    /// Read a committed value by key.
    pub fn read<T: Any + Send>(&self, key: &'static str) -> Option<&T> {
        self.ctx.get::<T>(key)
    }

    /// Render the mounted render-side components in mount order. Identical to
    /// rendering them directly—invalidation happens at the lifecycle layer, not
    /// by changing the cells.
    pub fn render(&self, width: usize) -> Vec<String> {
        self.renderers
            .iter()
            .flat_map(|(_, component)| component.render(width))
            .collect()
    }

    /// Expose the underlying kernel context to the host (e.g. for epoches).
    pub fn context(&mut self) -> &mut Context {
        &mut self.ctx
    }
}

#[cfg(test)]
mod tests {
    #![allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::arithmetic_side_effects
    )]
    use super::*;
    use crate::component::{Text, Component};
    use crate::terminal_image::{
        CellDimensions, TerminalCapabilities, snapshot_image_state, ImageProtocol,
    };

    fn render_of(text: &str) -> Arc<dyn Component> {
        Arc::new(Text::new(text, 1, 1))
    }

    /// Canon temporal check for the TUI substrate: snapshot, mount a component
    /// that commits editor state and scopes terminal-image resources, exercise
    /// every effect, unmount, diff — must be empty.
    #[test]
    fn unmount_reverts_every_tui_effect_with_no_residue() {
        // Snapshot the host state before mounting.
        crate::terminal_image::reset_capabilities_cache();
        let image_before = snapshot_image_state();
        let mut host = TuiHost::new();
        host.set(KEY_THEME, "dark");
        let theme_before = host.read::<&str>(KEY_THEME).copied();

        // Mount a component that commits a settings subscription, editor state,
        // and scopes the module-wide terminal-image resources.
        let editor_reactions = Arc::new(std::sync::Mutex::new(0usize));
        let editor_reactions_s = editor_reactions.clone();
        let id = host
            .mount(
                ScopedComponent::new(vec![KEY_THEME], render_of("prompt"))
                    .editor_state("idle")
                    .terminal_image_scope(),
            );
        let sub = host.mount_kernel(subscribe(
            KEY_SETTINGS,
            move |ctx, k| {
                assert_eq!(k, KEY_SETTINGS, "settings subscription got wrong key");
                let mut reactions = editor_reactions_s.lock().unwrap();
                *reactions += 1;
                let _ = ctx;
            },
        ));

        // Exercise every effect the mounted component committed.
        assert_eq!(
            host.read::<String>(KEY_EDITOR).map(String::as_str),
            Some("idle"),
            "editor state committed on mount"
        );
        // A live component mutates the scoped terminal-image resources while
        // mounted; the scope's inverse must restore them on unmount.
        let mutated = CellDimensions {
            width_px: 10,
            height_px: 20,
        };
        crate::terminal_image::set_cell_dimensions(mutated);
        crate::terminal_image::set_capabilities(TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: false,
        });
        assert_ne!(
            crate::terminal_image::get_cell_dimensions(),
            image_before.cell_dimensions(),
            "resource must be committed while mounted"
        );

        host.set(KEY_SETTINGS, "compact");
        assert_eq!(*editor_reactions.lock().unwrap(), 1);

        host.unmount(id);
        host.unmount(sub);

        // Diff is empty: image resources restored, editor value removed,
        // pre-existing theme preserved, subscription cancelled.
        assert_eq!(snapshot_image_state(), image_before, "image resource residue");
        assert!(
            host.read::<String>(KEY_EDITOR).is_none(),
            "editor state residue after unmount"
        );
        assert_eq!(
            host.read::<&str>(KEY_THEME).copied(),
            theme_before,
            "pre-existing theme must survive"
        );
        host.set(KEY_SETTINGS, "wide");
        assert_eq!(
            *editor_reactions.lock().unwrap(),
            1,
            "unmounted subscription must not react"
        );
    }

    /// Canon spatial check: change each declared key and confirm exactly its
    /// readers react; when a component declares none of a key, it must not.
    /// The rendered cells are unchanged throughout (parity oracle).
    #[test]
    fn spatial_invalidates_only_declared_readers_and_keeps_cells() {
        let mut host = TuiHost::new();
        host.set(KEY_THEME, "dark");
        host.set(KEY_SETTINGS, "compact");
        host.set(KEY_MODEL, "claude-3-5");

        let theme_inval = Arc::new(std::sync::Mutex::new(0usize));
        let model_inval = Arc::new(std::sync::Mutex::new(0usize));
        let nothing_inval = Arc::new(std::sync::Mutex::new(0usize));

        let t = theme_inval.clone();
        host.mount(
            ScopedComponent::new(vec![KEY_THEME], render_of("theme reader"))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_THEME, "theme reader got a non-theme key");
                    *t.lock().unwrap() += 1;
                }),
        );
        let m = model_inval.clone();
        host.mount(
            ScopedComponent::new(vec![KEY_MODEL], render_of("model reader"))
                .on_change(move |_, k| {
                    assert_eq!(k, KEY_MODEL, "model reader got a non-model key");
                    *m.lock().unwrap() += 1;
                }),
        );
        // This reader declares no read yet is mounted; it must never fire.
        let n = nothing_inval.clone();
        host.mount(
            ScopedComponent::new(vec![], render_of("no deps")).on_change(move |_, k| {
                *n.lock().unwrap() += 1;
                let _ = k;
            }),
        );

        let before = host.render(20);
        assert_eq!(before.len(), 9, "three render-side components render");
        assert!(before.iter().any(|l| l.contains("theme reader")));
        assert!(before.iter().any(|l| l.contains("model reader")));
        assert!(before.iter().any(|l| l.contains("no deps")));

        host.set(KEY_MODEL, "gpt-4o");
        assert_eq!(*theme_inval.lock().unwrap(), 0, "theme reader fired on model");
        assert_eq!(*model_inval.lock().unwrap(), 1);
        assert_eq!(*nothing_inval.lock().unwrap(), 0, "undeclared reader fired");

        host.set(KEY_THEME, "light");
        assert_eq!(*theme_inval.lock().unwrap(), 1);
        assert_eq!(*model_inval.lock().unwrap(), 1, "model reader fired on theme");
        assert_eq!(*nothing_inval.lock().unwrap(), 0);

        // Undeclared key: no reader at all reacts.
        host.set(KEY_SETTINGS, "wide");
        assert_eq!(*theme_inval.lock().unwrap(), 1, "undeclared key fired theme");
        assert_eq!(*model_inval.lock().unwrap(), 1, "undeclared key fired model");
        assert_eq!(*nothing_inval.lock().unwrap(), 0);

        // The committed sets invalidated readers only; the rendered cells are
        // byte-identical (parity oracle).
        let after = host.render(20);
        assert_eq!(before, after, "rendered cells must not change");
    }

    /// Generation / epoch: a consumer keyed by `[key, provider id, version,
    /// schema hash]` reloads only when its epoch changes.
    #[test]
    fn epoch_reloads_only_the_changed_generation() {
        let mut host = TuiHost::new();
        let schema = 0x9e37;

        let settings_reloads = Arc::new(std::sync::Mutex::new(0usize));
        let model_reloads = Arc::new(std::sync::Mutex::new(0usize));

        let sr = settings_reloads.clone();
        let sid = host.mount_kernel(subscribe_epoch(
            KEY_SETTINGS,
            Epoch::new(KEY_SETTINGS, "cfg-provider", 1, schema),
            move |_, _| {
                *sr.lock().unwrap() += 1;
            },
        ));
        let mr = model_reloads.clone();
        let mid = host.mount_kernel(subscribe_epoch(
            KEY_MODEL,
            Epoch::new(KEY_MODEL, "model-provider", 1, schema),
            move |_, _| {
                *mr.lock().unwrap() += 1;
            },
        ));

        // Same identity: no reload.
        host.set(
            KEY_SETTINGS,
            Epoch::new(KEY_SETTINGS, "cfg-provider", 1, schema),
        );
        assert_eq!(
            *settings_reloads.lock().unwrap(),
            0,
            "equal epoch reloaded settings"
        );
        assert_eq!(*model_reloads.lock().unwrap(), 0);

        // Version bump: only the settings consumer reloads.
        host.set(
            KEY_SETTINGS,
            Epoch::new(KEY_SETTINGS, "cfg-provider", 2, schema),
        );
        assert_eq!(*settings_reloads.lock().unwrap(), 1);
        assert_eq!(*model_reloads.lock().unwrap(), 0, "model reloaded on settings");

        // Schema hash change: only model reloads.
        host.set(KEY_MODEL, Epoch::new(KEY_MODEL, "model-provider", 2, schema + 1));
        assert_eq!(*settings_reloads.lock().unwrap(), 1);
        assert_eq!(*model_reloads.lock().unwrap(), 1);

        // Provider id change reloads its own consumer.
        host.set(
            KEY_SETTINGS,
            Epoch::new(KEY_SETTINGS, "other-provider", 2, schema),
        );
        assert_eq!(*settings_reloads.lock().unwrap(), 2);
        assert_eq!(*model_reloads.lock().unwrap(), 1);

        host.unmount(sid);
        host.unmount(mid);
        host.set(KEY_SETTINGS, Epoch::new(KEY_SETTINGS, "cfg-provider", 9, schema));
        assert_eq!(
            *settings_reloads.lock().unwrap(),
            2,
            "unmounted epoch consumer must not reload"
        );
    }

    #[test]
    fn epoch_structural_equality_matches_contract_fields() {
        let a = Epoch::new(KEY_THEME, "theme-provider", 3, 0x1234);
        let same = Epoch::new(KEY_THEME, "theme-provider", 3, 0x1234);
        let newer = Epoch::new(KEY_THEME, "theme-provider", 4, 0x1234);
        assert!(!a.is_stale_against(&same));
        assert!(a.is_stale_against(&newer));
        let _ = TerminalCapabilities {
            images: Some(ImageProtocol::Kitty),
            true_color: true,
            hyperlinks: true,
        };
        let _ = CellDimensions::default();
    }
}
