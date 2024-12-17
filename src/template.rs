use std::collections::{HashMap, HashSet};

use bevy_ecs::{component::ComponentId, prelude::*};
use bevy_hierarchy::prelude::*;

/// A template is an ordered collection of heterogenous prototypes, which can be
/// inserted into the world. Returned by the [`template`] macro.
pub type Template = Vec<Box<dyn Prototype + Send + Sync + 'static>>;

trait BuildTemplate {
    /// Builds a template onto the world.
    ///
    /// Each top-level prototype in the template will be built on a different
    /// entity. Each prototype's name is used to determine what entity to build
    /// it on, so naming root level entities is recomended. Unamed prototypes
    /// are indexed according to order. Different templates *will* conflict if
    /// they share the same root names or if root names are ommited on both.
    ///
    /// For information about what happens when a prototype is built on a
    /// specific entity, see [`Prototype::build`].
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

/// A command for building a template. The shorthand for this is
/// [`CommandsTemplateExt::build`]. See [`BuildTemplate::build`] for more
/// documentation.
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

/// Identifies data in a `Receipt` based on the positon and name of a
/// `Prototype`.
#[derive(Hash, Eq, PartialEq)]
enum Anchor {
    /// If a name is ommited from a the prototype, it will be given an
    /// incrementing id.
    Auto(u64),
    /// If a name is provided, we use that.
    Named(String),
}

/// A prototype is the type-erased trait form of a [`Fragment`] contained within
/// a [`Template`]. It has a name, and can be inserted into the world multiple
/// times, updating it's previous value each time.
///
/// This trait is mostly needed to get around `Bundle` not being dyn compatible.
pub trait Prototype {
    /// Returns the name of this prototype.
    fn name(&self) -> Option<String>;

    /// Builds the prototype on a specific entity.
    ///
    /// The prototype uses a receipt to keep track of the state it left the
    /// world in when it was last built. The first time it is built, it should
    /// use the default receipt. The next time it is built, you should pass the
    /// same receipt back in.
    ///
    /// The receipt is used to clean up old values after which were previously
    /// included in the template and now are not. Components added by the
    /// previous template but not the current one are removed. Children not
    /// added by the current template are despawned recursively. The children
    /// are also re-ordered to match the template.
    ///
    /// Where possible, this function tries to re-use existing entities instead
    /// of spawning new ones.
    ///
    /// To instead build an entire `Template` at the root level, see
    /// [`BuildTemplate::build`].
    fn build(self: Box<Self>, world: &mut World, receipt: &mut Receipt);
}

/// Receipts contain hints about the previous outcome of building a particular
/// prototype.
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
struct RootReceipt {
    receipts: HashMap<Anchor, Receipt>,
}

/// A fragment is a tree of bundles with optional names. It implements
/// [`Prototype`] and can be stored or used as a `Box<dyn Prototype>`.
pub struct Fragment<B: Bundle> {
    /// The name of the fragment, used to identify children across builds.
    pub anchor: Option<String>,
    /// The bundle to be inserted on the entity.
    pub bundle: B,
    /// The template for the children. This boils down to a type-errased
    /// `Fragment` vector.
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

// We implement this so that it is easy to return manually constructed a `Fragment`
// from a block in the `template!` macro.
impl<B: Bundle> IntoIterator for Fragment<B> {
    type Item = Box<dyn Prototype>;
    type IntoIter = core::iter::Once<Self::Item>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(Box::new(self) as Box<_>)
    }
}

/// This is a declarative template macro for bevy!
///
/// It gives you something a little like `bsn` and a little `jsx`. Like `bsn`,
/// it's a shorthand for defining ECS structures. Like `jsx` you can build
/// fragments (in this case `Template` values) at runtime and compose them using
/// normal Rust functions and syntax.
///
/// Here's an example of what it looks like:
///
/// ```rust
/// # use i_cant_believe_its_not_bsn::*;
/// # use bevy::prelude::*;
/// # let dark_mode = false;
/// # #[derive(Component)]
/// # pub struct MyMarkerComponent;
/// template! {
///     {(
///         Text::new(""),
///         TextFont::from_font_size(28.0),
///         if dark_mode { TextColor::WHITE } else { TextColor::BLACK }
///     )} [
///         { TextSpan::new("Hello ") };
///         { TextSpan::new("World") };
///         {( TextSpan::new("!"), MyMarkerComponent )};
///     ];
/// };
/// ```
///
/// The grammer is simple: Every time you see `{ ... }` it's a normal rust
/// code-block, and the template itself is just a list of fragments.
///
/// # Fragments
///
/// What's a fragment? Well, it's just a block that returns a `Bundle`, with an
/// optional name and list of child fragments. Names must be followed by a
/// colon, children are given in square brakets, and the whole thing always ends
/// with a semicolon. Behind the scenes these are used to create boxed
/// [`Fragment`] values.
///
/// # Splices
///
/// Templates can also have other template "spliced" into the list of fragments.
/// A splice is just a codeblock prefixed with `@` and returning a `Template`
/// (or more generally an iterator of `Box<dyn Prototype>>`). The contents of
/// this iterator is inserted into the list of fragments at the splice point.
/// Like fragments, splices also must be followed by a semicolon.
///
/// ```
/// # use i_cant_believe_its_not_bsn::*;
/// # use bevy::prelude::*;
/// let children_template = template! {
///     { TextSpan::new("child 1") };
///     { TextSpan::new("child 2") };
/// };
///
/// let parent_template = template! {
///     { Text::new("parent") } [
///         @{ children_template };
///     ];
/// };
/// ```
///
/// # Names
///
/// Fragments can be optionally prefixed by a name. A name is either a literal
/// symbols or code blocks that return a type implementing `Display`, followed
/// by a colon.
///
/// ```
/// # use i_cant_believe_its_not_bsn::*;
/// # use bevy::prelude::*;
/// let dynamic_name = "my cool name";
/// template! {
///     static_name:    { Text::new("statically named.") };
///     {dynamic_name}: { Text::new("dynamically named.") };
/// };
/// ```
///
/// Most fragments don't need names, and you can safely omit the name. But you
/// should give fragments unique names in the following three cases:
/// + Entities which only apper conditionally.
/// + Children that may be re-ordered between builds.
/// + Lists or iterators of entities of variable length.
///
/// Failing to name dynamic fragments will produce bugs and strange behavior.
///
/// # Limitations
///
/// This macro is fairly limited, and it's implementation is less than 50 lines.
/// You should expect to run into the following pain points:
/// + Each fragment must have a statically defined bundle type.
/// + The syntax for optional or conditional fagments is cumbersome (you have to use splices).
/// + You are responsible for ensuring dynamic fragments are named properly, and will not be warned if you mess up.
/// + It's hard to customize how templates are built, or build them on specific entities.
///
/// All of these can (and hopefully will) be addressed in a future version.
///
/// # Grammar
///
/// The entire `template!` macro is defined with the following ABNF grammar
///
/// ```ignore
///      <template> = *( <item> )
///          <item> = ( <splice> | <fragment> ) ";"
///        <splcie> = "@" <$block>                      -- where block returns `T: IntoIterator<Item = Box<dyn Prototype>>`.
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
        push_item!(fragments; $($tail)*);
        fragments
    }};
}

/// Used internally. See `template!()`.
#[macro_export]
macro_rules! push_item {
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
        $(push_item!($fragments; $($sib)*))* // Continue pushing siblings onto the current list.
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
                $(push_item!(fragments; $($children)*);)* // Push the first child onto a new list of children.
                fragments
            },
        };
        $fragments.push(Box::new(fragment) as Box::<_>);
        $(push_item!($fragments; $($sib)*))* // Continue pushing siblings onto the current list.
    };
}
