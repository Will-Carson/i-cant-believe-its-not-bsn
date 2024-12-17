//! This is a tiny declarative template library for bevy!
//!
//! The goal is simply to reduce the boilerplate of creating and updating
//! entities. To that end, this crate provides a handy `template!` macro for
//! describing dynamic ECS structres, as well as a simple `Template` struct for
//! holding templates as value. Templates can be built using `Commands::build`,
//! and they automatically update themselves if built multiple times.
//!
//! See the [`template`] macro docs for details.
//!
//! # Compatibility
//!
//! This module should not be mixed with the hierarchy module. Use one or the other, not
//! both.
//!
//! # Disclaimer
//!
//! This is a first attempt, and was written in about 48 hours over a weekend. There are
//! warts and footguns, issues and bugs. Someone more diligent or more knowlageable about
//! rust macros could probably significantly improve upon this; and if that sounds at all
//! like you I encurage you to try.
//!
//! The `template` macro is implemented declaratively instead of procedurally for no other
//! reason except that I am lazy and it was easier. A proc macro would probably be a better
//! choice.

use std::collections::{HashMap, HashSet};

use bevy_ecs::{component::ComponentId, prelude::*};
use bevy_hierarchy::prelude::*;

/// A template is an ordered collection of heterogenous prototypes, which can be inserted
/// into the world.
pub type Template = Vec<Box<dyn Prototype + Send + Sync + 'static>>;

trait BuildTemplate {
    /// Builds a template onto the world. The prototypes in the template are uniquely
    /// identified by name. The first time a name appears in a template, a new entity will
    /// be spawned. Rebuilding a template with a prototype for that name will update the
    /// existing entity instead of creating a new one.
    ///
    /// Building a template never despawns root level entities (that's your job), but will
    /// despawn children of template roots if they fail to match the template.
    fn build(self, world: &mut World);
}

impl BuildTemplate for Template {
    fn build(self, world: &mut World) {
        world.init_resource::<RootReceipt>();
        world.resource_scope(|world, mut root: Mut<RootReceipt>| {
            let mut i = 0;
            for prototype in self.into_iter() {
                // Determine the anchor for the prototype.
                let prototype_anchor = match prototype.name() {
                    Some(name) => Anchor::Named(name.to_string()),
                    None => {
                        let anchor = Anchor::Auto(i);
                        i += 1;
                        anchor
                    }
                };

                // Get or create the receipt for that anchor.
                let root_receipt = root.receipts.entry(prototype_anchor).or_default();

                // Build the prototype.
                prototype.build(world, root_receipt);
            }
        });
    }
}

pub trait WorldTemplateExt {
    /// Builds a template. See [`BuildTemplate::build`] for more documentation.
    fn build(&mut self, template: Template);
}

impl WorldTemplateExt for World {
    fn build(&mut self, template: Template) {
        template.build(self)
    }
}

/// A command for building a template. Created by [`CommandsTemplateExt::build`].
/// See [`BuildTemplate::build`] for more documentation.
pub struct BuildTemplateCommand(Template);

impl Command for BuildTemplateCommand {
    fn apply(self, world: &mut World) {
        self.0.build(world)
    }
}

pub trait CommandsTemplateExt {
    /// Builds a template. See [`BuildTemplate::build`] for more documentation.
    fn build(&mut self, template: Template);
}

impl<'w, 's> CommandsTemplateExt for Commands<'w, 's> {
    fn build(&mut self, template: Template) {
        self.queue(BuildTemplateCommand(template));
    }
}

/// Identifies data in a `Receipt` based on the positon and name of a `Prototype`.
#[derive(Hash, Eq, PartialEq)]
enum Anchor {
    /// If a name is ommited from a the prototype, it will be given an incrementing id.
    Auto(u64),
    /// If a name is provided, we use that.
    Named(String),
}

/// A prototype is the type-erased trait form of a [`Fragment`]. It has a name, and can be
/// inserted into the world multiple times, updating it's previous value each time.
///
/// This trait is mostly needed to get around `Bundle` not being dyn compatible.
pub trait Prototype {
    /// Returns the name of this prototype.
    fn name(&self) -> Option<String>;

    /// Builds the prototype on a specific entity.
    ///
    /// The prototype uses a receipt to keep track of the state it left the world in when
    /// it was last built. The first time it is built, it should use the default receipt.
    /// The next time it is built, you should pass the same receipt back in.
    fn build(self: Box<Self>, world: &mut World, receipt: &mut Receipt);
}

/// Receipts contain hints about the previous outcome of building a particular prototype.
#[derive(Default)]
pub struct Receipt {
    /// The entity this prototype was last built on (if any).
    target: Option<Entity>,
    /// The coponents it inserted.
    components: HashSet<ComponentId>,
    /// The receipts of all the children, organized by name.
    children: HashMap<Anchor, Receipt>,
}

/// A resource that tracks the receipts for root-level templates.
#[derive(Resource, Default)]
pub struct RootReceipt {
    receipts: HashMap<Anchor, Receipt>,
}

/// A fragment represents a hierarchy of bundles ready to be inserted into the ecs. You can
/// think of it as a named bundle, with other named bundles as children.
pub struct Fragment<B: Bundle> {
    /// The name of the fragment, used to identify children across builds.
    pub anchor: Option<String>,
    /// The bundle to be inserted on the entity.
    pub bundle: B,
    /// The template for the children.
    pub children: Template,
}

impl<B: Bundle> Prototype for Fragment<B> {
    fn name(&self) -> Option<String> {
        self.anchor.clone()
    }

    fn build(self: Box<Self>, world: &mut World, receipt: &mut Receipt) {
        // Collect the set of components in the bundle
        let mut components = HashSet::new();
        B::get_component_ids(world.components(), &mut |maybe_id| {
            if let Some(id) = maybe_id {
                components.insert(id);
            }
        });

        // Get or spawn the entity
        let mut entity = match receipt.target.and_then(|e| world.get_entity_mut(e).ok()) {
            Some(entity) => entity,
            None => world.spawn_empty(),
        };
        let entity_id = entity.id();
        receipt.target = Some(entity_id);

        // Insert the bundle
        entity.insert(self.bundle);

        // Remove the components in the previous bundle but not this one
        for component_id in receipt.components.difference(&components) {
            entity.remove_by_id(*component_id);
        }

        // Build the children
        let num_children = self.children.len();
        let mut children = Vec::with_capacity(num_children);
        let mut child_receipts = HashMap::with_capacity(num_children);
        let mut i = 0;
        for child in self.children {
            // Compute the anchor for this child, using it's name if supplied or an auto-incrementing
            // counter if not.
            let child_anchor = match child.name() {
                Some(name) => Anchor::Named(name),
                None => {
                    let anchor = Anchor::Auto(i);
                    i += 1;
                    anchor
                }
            };

            // Get or create receipt
            let mut child_receipt = receipt.children.remove(&child_anchor).unwrap_or_default();

            // Build the child
            child.build(world, &mut child_receipt);

            // Return the receipts
            children.push(child_receipt.target.unwrap());
            child_receipts.insert(child_anchor, child_receipt);
        }

        // Position the children beneith the entity
        world.entity_mut(entity_id).replace_children(&children);

        // Clear any remaining orphans
        for receipt in receipt.children.values() {
            if let Some(entity) = receipt.target {
                world.entity_mut(entity).despawn_recursive();
            }
        }

        // Update the receipt for use next frame
        receipt.components = components;
        receipt.children = child_receipts;
    }
}

/// We implement this so that it is easy to return manually constructed a `Fragment`
/// from a block in the `template!` macro.
impl<B: Bundle> IntoIterator for Fragment<B> {
    type Item = Box<dyn Prototype>;
    type IntoIter = core::iter::Once<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(Box::new(self) as Box<_>)
    }
}

/// # Purpose
///
/// This macro gives you something a little like `jsx`. Much like jsx lets you build and compose html fragments
/// at runtime using normal javascript functions, this lets you build and compose ECS hierarchy fragments
/// in normal rust functions, using normal rust syntax.
///
/// Here's an example of what it looks like:
/// ```rust
/// # use i_cant_believe_its_not_bsn::*;
/// # use bevy::prelude::*;
/// # let dark_mode = false;
/// # #[derive(Component)]
/// # pub struct MyMarkerComponent;
/// let template = template! {
///     {(
///         Text::new(""),
///         TextFont::from_font_size(28.0),
///         if dark_mode { TextColor::WHITE } else { TextColor::BLACK }
///     )} [
///         { TextSpan::new("Hello") };
///         { TextSpan::new("World") };
///         {( TextSpan::new("!"), MyMarkerComponent )};
///     ];
/// };
/// ```
///
/// The grammer is simple: The template contains a list of nodes, each with a name. Each node
/// may also have mote named nodes as children.
///
/// There is no custom syntax for logic. Every time you see `{ ... }` it's a normal rust code-block, and
/// there are several places where you can substitute in code-blocked for fixed values.
///
/// The most basic node is a block which returns a `Bundle`. This block can be prefixed with an optional name
/// annotation (eg `my_name: {( MyComponents ... )}`). A node may also have a list of child nodes
/// given after it in square brackets. All nodes must end with a semicolon.
///
/// You don't have to settle for a static structure either; instead of using the normal node syntax
/// you can just plop in a codeblock prefixed with `@` to insent a whole iterator of `Box<dyn Prototype>>`.
/// This is called "splicing".
///
/// # Names
///
/// Most nodes don't need names, and you can safely omit the name. But you should give nodes unique names
/// in the following three cases:
/// + Entities which somtimes there and somtimes not in different builds.
/// + Children that may be re-ordered between builds.
/// + Lists or iterators of entities of variable length.
///
/// # Composition
///
/// It's easy to compose functions that return `Templates`. See the examples directory for an indication
/// of how to do this. The simplest way is simply to return a template from a splice (`@{ ... }`) node.
///
/// # Grammar
///
/// The entire `template!` macro is defined with the following ABNF grammar
///
/// ```ignore
///      <template> = *( <node> )
///          <node> = ( "@" <$block> | <fragment> ) ";" -- where block returns `T: IntoIterator<Item = Box<dyn Prototype>>`.
///      <fragment> = <name>? <$block> <children>?      -- where block returns `B: Bundle`.
///          <name> = ( <$ident> | <$block> ) ":"       -- where block returns `D: Display`.
///      <children> = "[" <template> "]"           
///        <$ident> = an opaque rust identifier
///        <$block> = a rust codeblock of a given type
/// ```
///
#[macro_export]
macro_rules! template {
    ($($tail:tt)*) => {{
        #[allow(unused_mut)]
        let mut fragments = Vec::new();
        push_template!(fragments; $($tail)*);
        fragments
    }};
}

/// Used internally. See `template!()`.
#[macro_export]
macro_rules! push_template {
    // Handle the empty cases.
    () => {};
    ($fragments:ident;) => {};
    // Handle the case when no name is specified.
    ($fragments:ident; $block:block $( [ $( $children:tt )+ ] )? ; $( $($sib:tt)+ )?) => {
        push_fragment!($fragments; { None } $block $( [ $( $children )* ] )* ; $( $( $sib )* )* )
    };
    // Handle the fully specified case, when the name is a stiatic identifier.
    ($fragments:ident; $name:ident: $block:block $( [ $( $children:tt )+ ] )? ; $( $($sib:tt)+ )?) => {
        // Stringify the name and throw it in a code-block.
        push_fragment!($fragments; { Some(stringify!($name).to_string()) } $block $( [ $( $children )* ] )* ; $( $( $sib )* )* )
    };
    // Handle the fully specified case, when the name is also a code-block.
    ($fragments:ident; $name:block: $block:block $( [ $( $children:tt )+ ] )? ; $( $($sib:tt)+ )?) => {
        push_fragment!($fragments; { Some($name.to_string()) } $block $( [ $( $children )* ] )* ; $( $( $sib )* )* )
    };
    // Handle the case where it's just a codeblock, returning an iterator of prototypes.
    ($fragments:ident; @ $block:block ; $( $($sib:tt)+ )? ) => {
        $fragments.extend({ $block }); // Extend the fragments with the value of the block.
        $(push_template!($fragments; $($sib)*))* // Continue pushing siblings onto the current list.
    };
}

/// Used internally. See `template!()`.
#[macro_export]
macro_rules! push_fragment {
    ($fragments:ident; $anchor:block $bundle:block $( [ $( $children:tt )+ ] )? ; $( $($sib:tt)+ )?) => {
        let fragment = Fragment {
            anchor: $anchor,
            bundle: $bundle,
            children: {
                #[allow(unused_mut)]
                let mut fragments = Vec::new();
                $(push_template!(fragments; $($children)*);)* // Push the first child onto a new list of children.
                fragments
            },
        };
        $fragments.push(Box::new(fragment) as Box::<_>);
        $(push_template!($fragments; $($sib)*))* // Continue pushing siblings onto the current list.
    };
}
