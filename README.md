# i-cant-believe-its-not-bsn

An ergonomic ways to spawn Bevy entity hierarchies.

Eagerly [waiting for BSN](https://github.com/bevyengine/bevy/discussions/14437)?
Really wish you could spawn hierarchies with less boilerplate?
Just want to define some reusable widget types for `bevy_ui` in code?

This crate is here to help!

# Helper Components

This crate provides two helper components: `WithChild`, and its iterator sibling, `WithChildren`.

Just add it as a component holding the bundle you want to use to spawn the child, and you're off to the races.
A component hook will see that this component has been added, extract the data from your `WithChild` component, and then move it into a child, cleaning itself up as it goes.

These helper components are extreamly useful when you just want to insert a tree of entities declaratively.

# The Template Macro

Alternatively you can use the `template!()` macro, which is very similar to the proposed bsn syntax.
Arbitrary data can be passed into the macro using normal rust blocks.
The macro returns portable `Template` values, which can be spliced into other templates using `@{ ... }`.

Not only is the macro declarative and composable, it also supports basic incrementalization (doing partial updates to the ecs rathre than rebuilding from scratch).
Building the same macro multiple times with `commands.build(template)` does only the work nessicary to bring the ecs into alignmen with the template.
