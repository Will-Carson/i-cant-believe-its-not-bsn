use std::collections::HashSet;

use bevy_ecs::prelude::*;
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
    fn build(self, world: &mut World, entity: Entity);
}

impl BuildTemplate for Template {
    fn build(self, world: &mut World, entity: Entity) {
        for prototype in self.into_iter() {
            prototype.build(world, entity);
        }
    }
}

pub trait WorldTemplateExt {
    /// Builds a template. See [`BuildTemplate::build`] for more documentation.
    fn build(&mut self, template: Template);
}

impl WorldTemplateExt for World {
    fn build(&mut self, template: Template) {
        let entity_id = self.spawn_empty().id();
        template.build(self, entity_id);
    }
}

/// A command for building a template. The shorthand for this is
/// [`CommandsTemplateExt::build`]. See [`BuildTemplate::build`] for more
/// documentation.
pub struct BuildTemplateCommand(Template, Entity);

impl Command for BuildTemplateCommand {
    fn apply(self, world: &mut World) {
        self.0.build(world, self.1);
    }
}

impl EntityCommand for BuildTemplateCommand {
    fn apply(self, entity: Entity, world: &mut World) {
        self.0.build(world, entity);
    }
}

pub trait CommandsTemplateExt {
    /// Builds a template. See [`BuildTemplate::build`] for more documentation.
    fn build(&mut self, template: Template) -> EntityCommands;
}

impl<'w, 's> CommandsTemplateExt for Commands<'w, 's> {
    fn build(&mut self, template: Template) -> EntityCommands {
        let entity_id = self.spawn_empty().id();
        self.queue(BuildTemplateCommand(template, entity_id));
        self.entity(entity_id)
    }
}

pub trait EntityCommandsTemplateExt {
    fn build_to(&mut self, template: Template, entity: Entity) -> EntityCommands;
}

impl<'w> CommandsTemplateExt for EntityCommands<'w> {
    fn build(&mut self, template: Template) -> EntityCommands {
        self.queue(BuildTemplateCommand(template, self.id()));
        self.reborrow()
    }
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
    fn build(self: Box<Self>, world: &mut World, entity: Entity);
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

    fn build<'a>(self: Box<Self>, world: &'a mut World, entity: Entity) {
        // Collect the set of components in the bundle
        let mut components = HashSet::new();
        B::get_component_ids(world.components(), &mut |maybe_id| {
            if let Some(id) = maybe_id {
                components.insert(id);
            }
        });

        // Build the children
        let num_children = self.children.len();
        let mut children = Vec::with_capacity(num_children);
        for child in self.children {
            // Build the child
            let child_entity = world.spawn_empty().id();
            child.build(world, child_entity);
            children.push(child_entity);
        }

        // Get or spawn the entity
        world.entity_mut(entity)
            .insert(self.bundle)
            .add_children(&children);
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

/// This is a declarative template macro for Bevy!
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