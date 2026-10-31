# RFC: Feuer Logo and Visual Identity

**Status:** Proposed  
**Scope:** Logo concept, intended message, visual requirements, and acceptance criteria  
**Decision requested:** Agree on the creative direction before refining final artwork  
**Length:** Approximately five pages, depending on rendering

## 1. Summary

Feuer needs a logo that communicates **useful energy under control**. It should feel fast, capable, and memorable while remaining credible as the identity of an infrastructure library. Fire provides the connection to the name. A horse provides a recognizable character and a sense of purposeful movement. Together, they should express a system that puts resources to work efficiently.

This RFC proposes a flaming horse as the primary visual direction. The horse and fire should form one coherent silhouette, with flames growing naturally from the mane or shaping part of the head and neck. The result should be recognizable as a horse at ordinary display sizes and remain distinctive as a small icon. A simple wordmark reading **Feuer** should accompany the symbol when the project name needs to be introduced.

The central message is:

> Feuer turns repeated retrieval work into useful speed, making better use of memory, disk, and source access.

The logo should suggest that message through its character and composition. Documentation and supporting copy should explain the technical details. The mark should stand on its own without requiring a viewer to decode a diagram of the cache.

The repository already contains `horse_logo.svg` and `full_logo.svg`, identified as flaming-horse artwork. These are starting references for the proposed direction. This RFC establishes the criteria for evaluating and refining that direction. It does not approve the current artwork as the final identity.

## 2. Context: What the Identity Represents

Feuer caches immutable objects in memory and on disk. Applications supply callbacks for S3 and other sources. Its design includes cost-aware eviction, asynchronous SSD I/O, buffer reuse, and sharing of concurrent fetches for the same object. These capabilities serve a practical purpose: retain useful data and avoid repeating expensive retrieval work.

That purpose matters more to the identity than any individual implementation detail. A logo based on a particular storage device, kernel interface, or cloud provider would narrow the project’s meaning unnecessarily. Feuer supports memory-only operation, and its source callbacks are not limited to S3. The identity should remain appropriate across those uses.

The name offers a direct visual starting point: “Feuer” is German for “fire.” Fire suggests heat, energy, activity, and transformation. It also creates a useful association with frequently accessed, or “hot,” data. These connections make fire appropriate, but they introduce a tension. Fire can also suggest uncontrolled consumption, incidents, and destruction. Infrastructure users already spend enough time dealing with things metaphorically on fire.

The design must resolve that tension through restraint. It should show energy with a clear direction and a stable structure. The README’s line about burning an S3 budget can remain playful supporting copy. The logo should carry the more durable idea of making resources useful, rather than making waste or operational emergencies the defining image.

The intended audience includes developers evaluating the library, engineers integrating it, maintainers discussing its design, and readers encountering it in documentation or presentations. They should see a project with personality and technical seriousness. The identity can be lively without becoming a joke, and confident without implying performance guarantees.

## 3. What the Logo Should Communicate

### 3.1 Purposeful speed

The first impression should be movement with intent. A horse is a strong candidate because its posture can communicate speed, strength, and direction without adding a separate arrow or lightning bolt. A forward-facing or slightly advancing profile can suggest responsiveness while preserving a compact shape.

The desired feeling is “ready and capable.” A frantic expression, exaggerated motion blur, or scattered flames would suggest urgency and instability. Feuer should look like a component an engineer can depend on during ordinary operation, rather than an emergency intervention.

Speed is an emotional association here. The artwork must not imply that every request is a cache hit, that latency disappears, or that Feuer outperforms every alternative. Such claims belong in reproducible measurements with clear conditions.

### 3.2 Deliberate resource use

Feuer’s identity should also support the idea of efficiency. Its cost-aware retention decisions distinguish its purpose from a generic promise to make everything faster. The design should feel economical: a small number of meaningful shapes, an intentional silhouette, and little decorative clutter.

This does not require putting coins, price tags, or cloud billing symbols inside the mark. Those devices would turn a general cache identity into an illustration of one use case. Efficiency should come through in the visual discipline and be made explicit in nearby language when needed.

A suitable supporting phrase is **“Cost-aware tiered cache.”** It identifies the project category and a meaningful design characteristic. It should remain separate from the core symbol so the logo can be used without tiny explanatory text.

### 3.3 Composure and confidence

The logo should have a stable visual foundation. Balanced proportions, clear edges, and a readable expression should give the energy somewhere to go. The flames should belong to the horse, rather than appear to be consuming it.

This distinction is important. A horse whose mane becomes fire reads as a character with energy. A frightened animal surrounded by fire reads as a scene of danger. The former supports the project’s message. The latter undermines it.

Confidence in the visual identity should remain separate from storage guarantees. Feuer is a cache with best-effort disk caching and recovery. Shields, padlocks, and protective enclosures would introduce suggestions of security or durable data protection that are outside the logo’s purpose.

### 3.4 A recognizable personality

The identity should be memorable enough that someone can recognize Feuer after seeing it in a README and later encountering it in a presentation. A generic flame alone offers a clear connection to the name but little individuality. The horse gives the project a character that can survive changes in layout, color, and surrounding copy.

The intended personality is fierce, assured, and energetic. An aggressive edge can give the horse character and make it memorable. The symbol should be comfortable beside technical diagrams, code examples, and benchmark results.

There is room for occasional playfulness in supporting illustrations: the horse taking a kombucha break could work in a contributor slide or release announcement. Such scenes should remain separate from the core logo, where a clear silhouette matters most.

## 4. Proposed Visual Content

### 4.1 One horse-and-flame silhouette

The primary mark should combine a horse’s head and neck with a flame-shaped mane. A head-and-neck composition is the preferred starting point because it offers recognizable anatomy within a compact area. A full running horse may communicate motion more literally, but its legs and body introduce details that are harder to preserve at small sizes.

The horse should remain identifiable through the relationship between its ears, forehead, muzzle, jaw, and neck. Those features should work together rather than depend on detailed rendering. The flame treatment should reinforce the anatomy: a mane rising or sweeping backward, a small number of flame tips, and deliberate internal gaps where needed.

The design should read in this order:

1. A distinctive, cohesive symbol.
2. A horse with a fiery mane.
3. Optional secondary details visible at larger sizes.

It should avoid becoming a collection of separate icons. A horse, flame, cloud, database cylinder, and speed lines would compete for attention and weaken recognition. The combination of horse and fire already provides enough material for a complete identity.

### 4.2 Expression and movement

The face should convey intensity through shape and proportion. Bared teeth, flared nostrils, and sharply hostile eyes are welcome if they give the horse a fierce, defiant character. The distinction is between a horse commanding the fire and one distressed by it. An eye may help at medium and large sizes, but recognition should survive its removal.

Movement should come primarily from the angle of the head, the curve of the neck, and the direction of the mane. These cues should agree. Flames pointing in several unrelated directions would make the composition feel noisy rather than fast.

The symbol should maintain visual balance in a square crop. That does not require enclosing it in a permanent rounded square. A background tile can be useful for an avatar, while the underlying mark should also work independently on a page.

### 4.3 Color and light

A warm palette should connect the identity to fire. Orange can provide the main color, with red or gold used selectively to create depth or distinguish parts of the form. Dark neutral backgrounds may support presentation artwork, but the logo must also work on light documentation pages.

The final palette should include a small set of documented color values and approved foreground/background combinations. Pale yellow details on white need particular attention because they may disappear at ordinary viewing sizes.

Gradients are optional enhancements. The silhouette and internal structure must work in a single color. If removing the gradient makes the horse unreadable, the artwork needs structural refinement before additional rendering.

A flame does not need a glow effect to read as fire. Persistent halos, smoke, sparks, and intense lighting should be reserved for optional illustrations, if used at all. The everyday logo should remain crisp and easy to reproduce.

### 4.4 The Feuer wordmark

The standard name should be written **Feuer**, matching the project’s documentation. The wordmark should be readable and balanced beside the symbol. A straightforward typeface with enough weight to hold its own against the icon is preferable to distressed, scorched, or highly ornamental lettering.

Any custom letter treatment should preserve immediate recognition of the name. Turning several letters into flames would repeat the symbol’s message while reducing readability. A modest adjustment to spacing or one letter may be sufficient if customization genuinely improves the result.

The symbol and wordmark should look related through proportion and visual weight. They do not need matching decorative effects. When a descriptor is present, it should have a clear supporting role and remain removable without changing the main composition.

## 5. Required Forms and Everyday Use

The identity should be delivered in three practical forms:

| Form | Contents | Primary uses |
| --- | --- | --- |
| Symbol | Horse-and-flame mark | Repository avatar, compact navigation, favicon |
| Horizontal logo | Symbol and Feuer wordmark | README header, documentation, presentation title |
| Presentation banner | Horizontal logo with optional descriptor and background | Project introduction and larger promotional placements |

The banner is a presentation of the identity, not the only usable logo. Background panels, borders, and descriptive text should be separable from the symbol and wordmark. This keeps the identity useful when the available space or page background changes.

Each core form should support light and dark backgrounds. The symbol and horizontal logo should also have a single-color version. A reverse version should preserve important internal gaps rather than simply produce a solid shape with lost detail.

Small-size behavior should be considered during drawing, not postponed until export. Review the symbol at 16, 24, 32, 48, and 128 pixels. At 16 pixels, the goal is a clean, recognizable shape associated with the larger identity. At 32 pixels and above, the horse-and-flame reading should be clear. A simplified small-size symbol is acceptable when necessary, provided it preserves the same defining silhouette.

For the wordmark and banner, establish minimum display sizes from actual rendered examples. Avoid choosing a nominal minimum that leaves the name or descriptor unreadable. When space becomes too small for the descriptor, remove it. When the wordmark becomes unreadable, use the symbol.

Clear-space guidance should be simple and derived from the finished artwork. A single reference measurement, such as a fraction of the symbol’s height, is sufficient. The purpose is to prevent nearby text and graphics from crowding the mark, not to create a complicated brand manual.

## 6. Accessibility and Asset Quality

Recognition should rely on shape as well as color. The horse, mane, and important internal gaps must remain understandable in grayscale and in the single-color version. Color-vision simulations can help identify combinations that merge unexpectedly, particularly where red and orange define adjacent shapes.

Review every approved color version against its intended background. Supporting text should meet the applicable text-contrast requirements. Although logotypes receive exceptions under some accessibility standards, readable presentation remains a design requirement for this project.

Published images should have alternative text appropriate to their context. “Feuer logo, a horse with a flame-shaped mane” is suitable when the image introduces the project. If the same name already appears immediately beside a decorative image, empty alternative text may avoid repetition. When the image is a link, its accessible name should make the destination understandable.

Final source artwork should be editable vector geometry. An SVG wrapper around a raster image does not provide the same scalability or editability. PNG exports can accompany the SVGs for platforms that require them. Deliverables should preserve transparent backgrounds where appropriate and avoid unnecessary embedded data or editor metadata.

Record the source and usage rights of the artwork and typeface. If text is converted to outlines for distribution, retain enough source information to make future changes intentionally. The handoff should be small and usable: final assets, palette values, minimum-size examples, and a short usage note.

## 7. Alternatives and Trade-offs

### Flame only

A flame-only mark would connect directly to the name and simplify small-size rendering. Its main weakness is distinctiveness. Flame symbols are common across software and consumer products. This direction remains a fallback if the horse cannot be made readable, but it gives up much of the proposed identity’s personality.

### Full running horse

A complete horse would express motion strongly and could make an effective large illustration. It also creates a wide composition and delicate features that become difficult to read in an avatar. It is better suited to optional supporting artwork than the primary compact mark.

### Abstract cache or storage symbol

Layers, stacked shapes, and storage cylinders would describe the software category more literally. They would communicate less about the name and produce an identity that could belong to many infrastructure projects. Architecture diagrams can explain memory, disk, and source relationships more accurately than a logo can.

### Wordmark only

A wordmark would be straightforward and easy to use in documentation. It would provide less recognition in square and very small placements. The proposed combination retains the wordmark’s clarity while adding a symbol for those contexts.

The flaming horse is recommended because it balances a connection to the name, a sense of motion, and a distinctive character. That recommendation is conditional on successful simplification. Detail should be removed before accepting poor small-size performance.

## 8. Review and Acceptance

Review should start with monochrome sketches. This makes it easier to judge the underlying shape before color and lighting influence preferences. Compare a small number of genuinely different silhouettes, select a direction, and then refine its expression, palette, and typography.

Show candidates in realistic contexts: a repository avatar, a light README page, a dark documentation header, and a presentation title. Include actual-size small icons. Large artwork alone can conceal weaknesses that dominate everyday use.

Ask reviewers what they see before explaining the concept:

- What animal or shape do you recognize?
- Which three words describe its character?
- Does the fire appear to belong to the character or threaten it?
- Can you recognize the same mark at small and large sizes?
- What kind of software would you expect behind this identity?

These responses provide qualitative feedback, not a statistical study. Repeated confusion with a dragon, a distressed animal, or a generic gaming emblem should prompt revision. Reviewers do not need to infer cost-aware eviction from the image. That level of specificity belongs in the accompanying description.

The design is ready for adoption when:

1. The horse and fire form a coherent, distinctive symbol.
2. The dominant impression is energetic and composed.
3. The silhouette works in one color and remains recognizable at small sizes.
4. Light and dark variants preserve the important shapes.
5. The Feuer wordmark is readable and visually balanced with the symbol.
6. The symbol works independently of a banner, tagline, or background tile.
7. Source assets are editable, and artwork and font rights are documented.
8. A basic similarity review finds no obvious confusion with established identities in adjacent software categories.

The similarity review is a practical design check. It does not establish trademark clearance.

## 9. Decision and Next Steps

This RFC asks for agreement on three decisions: use a flaming horse as the primary direction, communicate purposeful speed and deliberate resource use, and make a simple reproducible symbol the foundation of the identity.

After agreement, evaluate the existing assets against these requirements. Preserve the parts that support the direction and simplify the parts that reduce clarity. Produce the symbol and wordmark first, then derive the presentation banner from them. Final review should use the acceptance checklist and realistic placements described above.

The remaining creative questions are the exact horse profile, the degree of flame stylization, and the wordmark’s typography. These should be resolved through visual comparisons rather than additional abstract requirements.

The intended outcome is a logo that makes Feuer easy to recognize and gives the project an appropriate character: **fast, focused, and resource-conscious, with fire as a source of useful energy.**
