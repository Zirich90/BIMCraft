# Structure and the discipline tabs (Plumbing, Electrical, SPDA, Data)

Moved from the root `CLAUDE.md`, word for word: the root keeps the rules and the map, and this file is read when the area is touched.

Structure (`runs.rs`, `structure.rs`; the design for the five discipline tabs is in [docs/disciplines.md](../disciplines.md)). A run
(`model::Run`) is a straight element from `start` to `end` in plan; its `offset` mm over its level is the height of its `justify` line
(top, centre or bottom) at the start, `end_offset` at the end (level when none), `system` the name of the system it is in: a beam, a
strip footing, a pipe (Plumbing), and what the other tabs draw (cable carriers, wires, down conductors, data cabling) as further
`Element` variants over the same struct, as `Door` and `Window` share `Opening`; `model::run_arm!` is the pattern for any of them, so a
place that treats runs alike names no category, and a new category is a line there. Its type is `Types::runs[category][name]`, a `RunType`
(`width` across by `depth` deep, or round, in a `material` of the library). The section is square to the axis, so a sloped run is
`depth / cos θ` deep vertically (`Geom::half`). `runs::Geom` is the one place that knows the geometry (centre line, `footprint`,
`prism` for a rectangle, `tube` for a round one, `z_span`); plans, sections (a prism is cut like any solid; a tube by the plane
through the section's line, `Solid::cuts`: the hull of where its edges cross it, a ring, an ellipse when oblique), 3D, snaps and IFC call
it and know no category. A run's heights are those of its `justify` line, which a beam or a footing has at the top by default and
anything else at the middle (`runs::default_justify`); the desktop keeps each run tool's own justification and offset
(`keep_run_options`). A beam or strip footing can't stand or lean past 75°
(`runs::check`, `may_stand`: what stands is a column; a pipe, a cable carrier or a conductor may stand, a riser). In plan (`render::stands`) a footing or a run
hangs under the level it is drawn on, its top flush with the view's bottom, and is seen (`seen_below` allows `TOL` for these only,
or a wall of the storey below would show in the plan above); through the cut plane it's cut; over it and under the view range's top
it's dashed; a faint dashed centre line is drawn where it's seen or overhead, which is also where a click picks it. A structural column and an
isolated footing are `Column`s in their own categories (`Element::StructuralColumn`, `Element::Footing`; ask `Element::column()`,
`Model::column_type(e)` and `column_outline(e)`, which take the element, not the column) with types of their own
(`Types::structural_column`, `Types::footing`); a footing is laid down, its `height` its thickness and its `base_offset` putting its
top at the level (the command's `thickness` and `offset`). A floor is `structural` or not, the one flag, and its schedule and IFC say it.
`runs_of` and `stood_of` build what a command would make, so a tool's sketch is the element its click makes. The desktop's
`Tool::Run(category, start)` and `Tool::Stand(category)` serve every category: a tab adds a ribbon button and its rows in
`structure.rs`, `properties.rs` and `lang.rs`, not a tool. The Floor tool makes structural floors while `floor_structural` is set (SB,
the Structure tab's button; every other `set_tool` clears it); CL is the structural column on the Structure tab and the architectural one
elsewhere, as in Revit. IFC writes a run as an extrusion along its own axis (`Step::body_along`, `aim`) of the class and predefined type
the one table `runs::IFC` gives its category (import reads the same table back, `runs::from_ifc`), each structural element with
`LoadBearing` in its `Pset_*Common`, a run's system as an `IfcSystem` (named for it, its GlobalIds following the name) the runs in it
are assigned to by an `IfcRelAssignsToGroup`, its `Justification` and `Category` in `BIMCraft_Run`, and its material as an `IfcMaterial`.
Import (`import::run_in`) finds a run's axis as the direction its body's points spread furthest in (power iteration on their
covariance), so a beam rolled about its axis comes in as the box round it; an axis that stands (a riser) has its width along east and
runs up; the justification BIMCraft wrote puts the heights back on the line they were typed at (`Geom::half`), none meaning the middle;
a system comes from the IfcSystem the product is assigned to, and its category from the class and type, or BIMCraft's `Category`
where a class has several; `LoadBearing` makes a column or a floor structural.

**Sizing** is a command with no model behind it, so it needs no connectors: `circuit` (`nbr5410.rs`, its tables in `nbr5410/tables.rs` as NBR 5410:2004 prints them: Tabelas 36 and 37 current capacity for methods A1 to D, 40 and 41 temperature and soil, 42 and 45 grouping, 47, 48 and 58) sizes one electrical circuit for copper: the design current from watts or VA, the demand and power factors and the phases at the voltage `system` it is given (it reads nothing from the model), then the smallest section and breaker with I_B ≤ I_n ≤ I_Z (which gives I_2 = 1.45 I_n ≤ 1.45 I_Z for the breakers of NBR NM 60898) and a voltage drop within the limit (4 %, from the conductor's resistance at its working temperature and 0.1 ohm/km), with the neutral and the protective conductor. A section or breaker already chosen is never made smaller. It does not check aluminium, trays (methods E and F), several layers, short circuit, harmonics or the conditions of a reduced neutral, and says so; a circuit in the model and a panel schedule come after it. `conduit_fill` (`conduit.rs`, its catalogues in `conduit/tables.rs`) checks a conduit's fill: the conductors of a circuit (phases, neutral, protective; by outside diameter) over the conduit's inside, at most 53 % for one cable, 31 % for two and 40 % for three or more (NBR 5410), and the smallest size of a family that takes them. The dimensions are the makers' own (Amanco Wavin PVC, Elecon steel, Corfio flexible cable), each family naming its document, page and address; the inside diameter is derived (outside less twice the wall); a cable the catalogue lacks (EPR, above 240 mm²) leaves the fill pending, never guessed. Trays are not sized. `drain_size` and `vent_size` (`drainage.rs`) size sanitary drains and vents as NBR 8160 asks, by the fixture units (UHC) they carry, with the standard's own tables: Tabela 3 (the units and smallest drain of each fixture), 5 (branches), 7 (building drains and collectors by slope), 6 (stacks by the building's floors), 8 (branch vents), 2 (vent stacks and headers by the DN served, its units and the longest length of each vent DN) and 1 (a trap's longest run to its vent). A drain is never smaller than its fixtures' own drains or than one flowing into it (`min_dn`), and a building drain is DN 100 at least. They are stateless, so none of this needs a pipe network; a network that carries the units along the runs comes with connectors. `storm_size` (`storm.rs`) sizes storm drains as NBR 10844 asks: the flow Q = I·A/60 from a rainfall and a roof area, a downpipe by the inner diameter of the standard's Figure 3 abacuses (read curve by curve: the water depth in the gutter, the downpipe's length and a sharp-edged or funnel outlet; 70 mm inside at least; off the chart nothing is read) and a horizontal drain by Manning at two thirds full (n 0.011, which reproduces its Table 4 within 1 %; 0.5 % slope at least). `water_size` (`water.rs`) sizes a cold water pipe of PVC soldável as NBR 5626 asks: the design flow of the fixtures by the method the project chooses (weights, Q = 0.3 √ΣP, or every fixture at once), the smallest DN within 3 m/s, the Darcy-Weisbach head loss over the length and the equivalent length of the fittings, and the pressure left (10 kPa at least at a point of use). `hydrant_size` (`hydrant.rs`) sizes hydrants and hose reels as IT 22/2025 of the CBPMESP asks: the system type and fire reserve of Tabela 3 by occupation column and built area, the flow of the two most unfavourable jets of Tabela 2's type, and a pipe's speed (5 m/s at most), Hazen-Williams head loss (Tabela 1) and the head needed at its start. `load_points` (`loads.rs`) lists what a dwelling's rooms must have as NBR 5410:2004 item 9.5.2 asks: one ceiling lighting point per room (100 VA up to 6 m², 60 VA more for each whole 4 m² above) and the general-use outlets by the room's kind, which its name says or `kind` gives (a bathroom 1; kitchen, pantry, service area and laundry one for each 3.5 m of perimeter; room and bedroom one for each 5 m; veranda 1; the others 1 up to 6 m² and by perimeter above), with their power (600 VA each for the first 3 points of a bathroom, kitchen, pantry, service area or laundry and 100 VA after, or for the first 2 by the alternative where those rooms have more than 6 points together; 100 VA a point elsewhere) on the points a room has. It reads the model's rooms (inside the walls' finish) or the rooms it is given, and changes nothing. It leaves out the exceptions to the lighting point and to a veranda's outlet, where the outlets stand, and the TUE, and says so.

**Electrical settings** (`electrical.rs`) are the tables circuits, panels and connectors will name, as Revit's Electrical Settings: `Voltage` (`nominal`, and the band `min` to `max` a device may read within), `DistributionSystem` (`phases` 1 to 3, `wires` the line conductors and the neutral and not the protective one, and the voltages it supplies by name: `l_l` between lines, `l_n` to the neutral, `l_g` to earth, and `ground`, how it is earthed as IEC 60364-1 names it) and `WireType` (`material`, `temperature`, `insulation`, a `note` saying where the ampacities come from, and `sizes`, a row for each section with its `ampacity`). Every section is in mm² whatever the project's units; a size a standard gives in AWG or kcmil is converted and keeps its name as `label`. Each is a map of `Model` (`voltages`, `distribution_systems`, `wire_types`), a library entry (`Kind::Voltage`, `Kind::DistributionSystem`, `Kind::WireType`, so each has an id and a GUID, comes in with Insert from File and is renamed with `rename_entry`), and has a command that makes, edits, renames and deletes it and, without a `name`, lists it (`voltage`, `distribution_system`, `wire_type`); `inspect` lists all three. A system names voltages, so `Model::lib_refs_mut` hands them out (a file holds their GUIDs, a rename follows) and `Model::check` refuses a system that names one the project lacks, saying which and listing the ones it has, and `voltage` refuses to delete one a system supplies; with three phases `l_l` must be about root three times `l_n`. `Model::voltage_range`, `system_voltages` and `wire_ampacity` are what circuits will ask: the band of a voltage, the voltages a system supplies (the ones a circuit on it may have) and the ampacity of a section in a wire type's table. A new project starts with `data/electrical.json`, data and not code: 230/400 V, 127/220 V, 220/380 V and 277/480 V, and copper wires of NBR 5410:2004 Tabelas 36 and 37 (PVC 70 °C and XLPE or EPR 90 °C, seeded from `nbr5410/tables.rs`: method B1, two loaded conductors, a test keeps them equal) and of NFPA 70 Table 310.16 (THHN 90 °C, THW 75 °C); their ampacities are the contributor's and unchecked, and one number for a section cannot say how a wire is laid, so a wire type's `note` says. `start_system` and `start_wire` name the ones a new panel or circuit starts with for the project's units (Europe's in metric, North America's in feet and inches). None of it goes to IFC: the tables are project data, and what they produce (voltage, current, section) will go in the device's property set once circuits exist. On the desktop, Manage > Electrical Settings (`ui-egui/src/electrical.rs`) is one dialog with a tab for each table: rows to add and delete, a voltage chosen from the project's own for each line of a system, and a wire type's table of sections below its fields. It is a draft, applied by OK or Apply as one undo step (`Session::transaction`) through the three commands, in an order the core accepts (`renames::plan`, which the other Manage dialogs use, orders each table's renames and deletes; the voltages are made before the systems that name them and taken away after the systems that no longer do). A system's voltages are held by their rows' keys, so one renamed in the dialog is still the one named. A blank or repeated name, a voltage a system supplies being deleted, and what the core refuses are said inside the dialog, which stays open. Not yet: circuits, panels and the load schedule, connectors in families.

The discipline tabs after Structure add data to this, no engine code of their own. Plumbing: `pipe` (PI), types in PVC and copper
(`Types::runs["pipe"]`, outside diameters, so a pipe's `width` is its diameter and its schedule says Diameter). Electrical:
`cablecarrier` (`cable_carrier`, CC; Cable Carrier / КНС) and `wire` (EW), by their outside size. One carrier category holds round
conduits and rectangular trays and channels; `runs::ifc_shape` writes them as `IfcCableCarrierSegment` of predefined type
`CONDUITSEGMENT` or `CABLETRAYSEGMENT` according to their section. Upstream conduit, cable tray and duct categories remain available. Lightning protection (NBR 5419):
`downconductor` (DC, may stand: a riser down a wall) and `groundconductor` (GC), two runs of one IFC class that `BIMCraft_Run`'s `Category`
tells apart; an air terminal, an earthing electrode and an inspection box are components of their own family categories, written as
`IfcBuildingElementProxy` of the object type `Category::ifc` names, so the product's name (not its object type) holds `Family:Type` and
import reads it from there. Data (NBR 14565): `cable` (CB, `cabling`), a run like a wire, so a file's `IfcCableSegment` is a wire unless its
`Category` says cable; data and voice outlets (`IfcOutlet` `DATAOUTLET`, `TELEPHONEOUTLET`; one of no type reads as the power outlet,
`Category::from_ifc` taking the first of equals), a patch panel (`IfcCommunicationsAppliance`) and a rack (a proxy of object type `Rack`);
the Cable Carrier tool is the Electrical tab's, on the Data tab too. Fire and security (SP 484, SP 3.13130): a fire alarm panel
(`IfcUnitaryControlElement` `ALARMPANEL`), smoke, heat and motion detectors (`IfcSensor` `SMOKESENSOR`, `HEATSENSOR`, `MOVEMENTSENSOR`;
one of no type reads as the smoke detector), a manual call point, a sounder and an exit sign (`IfcAlarm` `BREAKGLASSBUTTON`, `SIREN`,
`LIGHT`), a ceiling speaker and a camera (`IfcAudioVisualAppliance` `SPEAKER`, `CAMERA`), `fixtures::fire_and_security`; the Low Voltage
tab places them, and the Component tool too (FA, DS, HD, CP, SR, ES, AU, MO, VC). A fixture or a
device is a component of a family whose `family::Category` says what it is (a sanitary fixture, a floor drain, a pipe fitting, a pipe
accessory…); `Category::ifc` is its IFC class, predefined type and, where IFC has no class (lightning protection), the object type a
proxy carries, and `Category::from_ifc` reads the same table back, so a category is one row, in `family.rs`. The starter families are
`fixtures.rs` (original forms, named in the project's language; a fitting or a valve is centred on its pipe, along x, so placing it at the
pipe's height lines them up; a wall device, a board and a luminaire carry their `Mounting Height` in their type, so the Fixture tool
places them at the level and they stand where they hang). The desktop's `mep.rs` has one tool for every fixture and device, `Tool::Fixture(key)`: the families of the
kinds its `FixtureTool` names, listed in its Type Selector and placed as the Component tool places them (`placing` takes the tool's
kinds), and the tabs' ribbon panels; a fixture's two-letter shortcut is in `mep::FIXTURES`, not `SHORTCUTS`, since no command makes
one. Edit Type of a run type also sets whether it is round (a diameter, no depth) and its material.

`Element::Wire` (`wire`, EW) is both the physical cable and its optional logical connection: a `source` component and ordered
`devices`, by GUID, with `system` as its loop or panel label, `cable` as its make and `reserve` as a percentage.
`along_walls` (default false, `modify` reads it too) takes each leg along the level's walls instead of square to the axes:
`Model::wall_leg` runs the cable along the walls' location lines, going from one wall to another where their ends meet (within
2×`TOL`), by the cheapest way (`MAX_WALL_ROUTE` walls at most, square legs outside that); the hook from a device to the wall
it leans on stays straight. `Model::circuit_route` gives these logical legs; `Model::circuit_length` sums them and the height
changes between `Model::device_point`s, with reserve. A physical-only wire reports its axis length. The `wire` schedule offers
Source, Devices and Cable beside the ordinary run fields; Name is the system label. Device addresses (`Model::address`) use
that label and the device's position, e.g. "L1.3". Plans and IFC draw the physical axis, as `IfcCableSegment`.
`carrier` optionally names a Cable Carrier by GUID; `local_offset` is stored on the wire. Deleting a carrier detaches its wires;
deleting a source or the last device clears the logical connection while retaining the physical wire.

### Physical equipment connections (stage 4)

`cable_carrier {from_wire: id|GUID, type?, placement?: centre|top|bottom, local_offset?}`
creates a carrier around the selected wire's actual axis, including its slope or
vertical span, and links the wire by GUID in the same undo step. Placement uses
the section's local frame, keeping the wire fixed; its whole section must fit,
including rectangular corners inside a round carrier. Already linked wires and
owned equipment feeders are refused. Create Carrier from Wire opens an explicit
type, placement and lateral-offset editor in plans and the 3D view.

The run Options Bar exposes End Offset (blank means the start height) and Vertical
for wires and carriers. Vertical fixes the second point's plan coordinates at the
first point. A chained segment starts at the preceding segment's end height. In
3D, Work Level and Offset define the click plane; equipment has its own Offset.
Switch to 2D/3D keeps the active placement tool and remembers the work level.
Drawing a carrier over suitable wires offers their explicit selection and
confirmation; neither a unique nor an ambiguous overlap attaches silently.

`wire_connect {wire, device, point: [x,y,z], in_carrier?}` records an explicit point
relative to the equipment's insertion point, rotated and mirrored with that
instance. It projects onto the wire in plan (onto its height span for a vertical
wire), creates the vertical leg followed by the horizontal leg, and omits zero
length legs. A sloped wire supplies the connection height at that plan position.
`in_carrier` requires the main wire to have a carrier and creates matching carrier
legs with its type. This explicit point precedes the ports planned for stage 5.

The main `Run.connections` holds equipment GUIDs and local points; generated
`Run.feeder` names the main wire, equipment and leg role. All are visited GUID
references. Session regeneration updates surviving roles in place, preserving
their ids and GUIDs, rather than appending another set. Each physical wire leg's
reported length includes the main wire's reserve once; generated carrier lengths
have no cable reserve. Deleting the main carrier detaches the connection and
removes its owned carrier legs while retaining the physical wires and their GUIDs.
Format 14 protects this ownership contract from older
readers. Stage 4 remains in progress; these commands do not establish completion
of the full floor workflow or its visual acceptance.

The desktop's Wire button on Electrical, Data and Low Voltage uses `Tool::Run("wire", ...)`, the same point-to-point tool as
other runs. Logical references are set through the `wire` and `modify` commands, by id or GUID. The old separate Circuit tool,
CI shortcut and Divide Panel command have been removed. Format 8 migrates old carrier categories and logical loops to the new
elements, preserving their ids and GUIDs; an old loop's physical axis starts at its source toward its first distinct device.

`detectors` lays point fire detectors over a room (SP 484.1311500.2020): `Region::cover` puts a square grid over the room's
bounds, its cells' diagonal twice the detector's reach (a smoke detector's 6400 mm, a heat detector's 3500, closed in
proportion when the room's ceiling, to the level above, stands higher than the family's mounting height, or the `radius`
given), and a detector in each cell the room has, at the cell's centre or the point of the cell nearest it that is inside and
`margin` (500) off the walls. A rectangle is covered exactly; a cell an edge cuts is covered from its part inside; a `radius`
whose grid would ask for over 2000 detectors is refused. It leaves to the designer what SP 484 asks besides: two detectors
where one alone would trip the alarm, and beams and ducts that split a ceiling.

Panel-limit automation is a future Wire workflow; panel families still describe their capacity in their type names.

`wire_connect` accepts `end: 0|1` (default `0`). A cable returning to the same
equipment retains two independent connections and sets of owned feeder legs.
Updating one end preserves the other and keeps the surviving legs' ids/GUIDs.
The desktop connection dialog selects the first or second end and loads its
existing draft. This identifies physical ends, not logical terminals. FORMAT15
defaults FORMAT14 connections and owners to end zero; older readers refuse the
new format rather than conflating the ends. Allowed device sides and wall-surface
tangency remain separate work; explicit local points are still the current solver.

`wire_connect` also accepts an optional `carrier` id/GUID for that end's route.
Omitted/null reconnects to the main wire route; an explicit carrier can be chosen
independently for each end, including four ends of two returning cables. The
dialog selects this route separately from whether to create carrier branches.
Refresh stores the last world-space landing and follows the selected carrier.
Deleting it removes that link and the generated carrier branches, preserving the
physical cable's landing rather than jumping it onto another route. Reconnecting
chooses a new route. Copy/transform remap route GUIDs and transform a detached
landing; surviving feeder roles keep their identity. FORMAT16 defaults the new
fields to absent for old files and prevents older readers discarding a route.
The landing is a route endpoint, not a wall attachment or bend-radius proof.

Stage 3 links a Wire to its CableCarrier by GUID. Translation, rotation and mirror
move the cable once, including when both are selected. Direct axis/height edits
map cable centres through the carrier's 3D frame. Type depth changes, including
closed-project ODS synchronization, retain cable position relative to the physical
section for top/bottom justification. Undo/Redo preserves both instances and GUIDs.

`open` after ODS synchronization and `inspect` report peak simultaneous gross cable
section along carrier intervals. `cable_carrier_check` takes explicit `fill_percent`
and `mode: warn|auto`; `configure` saves the policy and optional project default
carrier type. Auto chooses the smallest sufficient shape-compatible type from the
same known manufacturer for a straight connected run, including across perpendicular
branches. It checks both peak area and the cable's offset/oriented envelope, keeps
one size along the run, and never reduces an existing section's area. Unknown
manufacturer or insufficient catalogue leaves warnings. Persisted `size_locked`
marks a manual size boundary excluded from automatic sizing; locked overfill is
reported. Advisory checks do not add Undo steps.

No regulatory percentage is assumed. Gross type dimensions are not verified internal
dimensions. Instance `compartments` record measured usable width/depth/lateral offset
and explicitly permitted systems; assigned cables receive per-compartment geometry,
separation and simultaneous-fill warnings. These measurements must fit any candidate
type. They describe constraints, not independently identified divider products.
Manage > Equipment Catalogue exposes the policy, project type, manual locks and
selected element's compartment settings. Checks highlight affected carriers;
opening and ODS polling retain the feedback. See `docs/stage-3-progress.md` for
current acceptance evidence and the pending Russian regulatory profile.
# Surface query for mounting drafts

`run_mount {id, host, face, revision?}` persists round-run contact or a rectangular cable carrier's mounting back on a
straight wall (`face: exterior|interior`), floor or ceiling (`face: top|bottom`).
The host key is required; explicit
`host: null` detaches while preserving the current physical axis and section orientation. `revision`
rejects stale proposals. FORMAT 18 stores optional `Run.mount` with the host
GUID. Mounted endpoints and heights describe the contact line; shared run
geometry derives the centre line by adding the radius along the surface's outward
normal. Changing diameter therefore preserves surface contact. A rectangular
carrier instead shifts its axis by half its depth, with its depth axis along
the outward normal and its width along the surface. Its entire mounting back,
including openings away from the centre line, must be supported. `section_roll`
retains the section orientation after detaching or deleting the host; absent
means zero. `Geom::frame` is shared by candidate, envelope and compartment
checks, the rolled box mesh and the IFC sweep's reference direction. Legacy runs
without this field retain their existing axis/justification semantics.

FORMAT 19 also stores `Run.square_ends` (default false, omitted when false).
Mounted rectangles use end planes normal to the run axis even when their
section roll is zero. The same mode selects the physical box in Scene, plan
footprint and vertical bounds; detaching or deleting the host persists it.
A legacy unmounted rectangle without this flag keeps its plumb-cut prism.
The version fence prevents older readers silently changing a detached,
sloped carrier's shape. Round-run geometry remains unchanged by this flag.

Rotation and reflection also transform an unmounted rectangular run's physical
section. Upright sections use a global reference axis, so their roll must change
when the run rotates. Reflected physical boxes retain their end-plane mode even
if their new roll is zero. Unrolled legacy sloped prisms keep their existing
plumb cuts; mounted sections continue to derive orientation from their host.
The shared command transformation serves both edits and copies.

The whole contact segment must lie on the named face: triangle-interval union
rejects a segment crossing an opening even if both endpoints are supported.
Host motion follows the old host frame, including level elevation exactly once.
Copy remaps a copied host; deleting the host detaches at the previous physical
axis. Creating a carrier from the mounted wire transfers ownership to that
carrier and preserves physical wire geometry. These cascades share the command's
Undo step. Save/Open and deletion Undo/Redo have command-level regression tests.

Unchanged linked cables follow a mounted carrier's physical frame when its host
moves or its section depth changes. Explicitly edited cables take precedence.
Their physical endpoints and section orientation are mapped together in the
same command, before the owned connection legs are refreshed.

This implementation accepts independent round MEP runs and rectangular cable
carriers. Floor mounting uses the actual sloped plane and vertical slab depth;
ceiling mounting uses its underside and type thickness. The host frame is
anchored at the first boundary vertex with a lifted boundary tangent. Both
tangential coordinates follow translation, rotation and slope changes; the
finite face, holes and shafts still determine support. Slab top/bottom names
follow the sign of the face normal, including allowed slopes above 60 degrees.
Curved walls and roofs remain outside the run mounting solver.

`surface_query {host, origin:[x,y,z], direction:[x,y,z], reach, revision?, footprint?}` is a
read-only core command for a selected wall, floor, roof or ceiling. Coordinates,
reach and returned distance are in mm. Direction is normalized; parameters are
strict and bounded. The response has a session `revision` and `hit`, either null
or `{host: GUID, id, face, point, normal, distance}`. Normals face outwards and
semantic face names (`exterior`, `interior`, `top`, `bottom`) are independent of
triangle indices. An optional outdated revision is refused.

This host-specific query uses the scene's actual triangles and openings; other
objects do not occlude the selected host. Curved faces give tessellated normals.
An optional footprint is four ordered world-space corners of a rectangle in the
hit plane. The response then adds `supported`: whether its whole area is on that
named planar face (false on ray miss). `Scene::supports_patch` subtracts the union
of coplanar triangles, so holes inside a rectangle and overlapping triangles are
handled. Degenerate/nonplanar rectangles are refused; a fragmentation/work limit
returns an error rather than guessing support. Numerical tolerances are 1e-6 mm
for coplanarity and 1e-8 square mm for fragment area. This is planar support on
tessellated geometry, not exact support on a curved surface.
Rectangle shape and coordinate validity are checked even on a ray miss.

The query does not persist an attachment. Drafts must query again after model edits. `Session::revision` is an
ephemeral snapshot token backed by a weak model reference, independent of file
format and Undo history.

`component` and `modify` accept `connection_sides` with any combination of
`bottom`, `top`, `right` and `left` in equipment coordinates. An empty list
allows all four. To use a free contact, call `wire_connect` with
`surface: {host, face}` on a straight wall (`interior` or `exterior`). The
existing `point` input still selects a fixed local contact.

The solver chooses a contact along the allowed sides of the equipment envelope
and a landing on the finite route. It then offsets the cable axis by the outside
radius along the wall normal. Each connection end can name a separate carrier.
Its straight lead stays tangent to the wall and must fit the selected carrier
section at the landing. Openings that interrupt the lead cause the command to
roll back. This check covers the straight lead and landing geometry. Bend radius
verification at the transition remains pending.

Reinforcement (`rebar.rs`, `model::Rebar`, `Element::Rebar`, category `rebar`, the `rebar` command; tests in `rebar_tests.rs`). A bar is one
element: a round bar of `diameter` mm (3 to 100) bent along `path` (points in mm, `z` over its level), of steel `grade` (CA-50 unless
told), with the `host` it reinforces (a beam, a column, a footing or a floor, held as a `Guid`, and it goes when its host does). A hoop
is a bar whose last point is its first; a layer of bars, or hoops along a beam, is one command with `count` and `step`, which makes one
element for each (IFC has one `IfcReinforcingBar` for each, and a bar schedule counts them). It has no type: the diameter and the grade
are its own, and the bends are sharp, with no hooks, so its length is the sum of its pieces (`rebar::length`) and its weight that
length of steel at 7850 kg/m³ (`rebar::weight_kg`). Nothing checks that it lies inside its host: `reinforce` places bars inside by construction. The schedule of category `rebar` lists Id, Level, Grade, Diameter,
Length, Count, Weight and Host, and grouped by Grade, Diameter and Length with Count and Weight totalled it is a bar schedule. IFC
writes each as an `IfcReinforcingBar` (`SteelGrade`, `NominalDiameter`, `CrossSectionArea`, `BarLength`) with a body of an
`IfcSweptDiskSolid` along an `IfcPolyline`, aggregated (`IfcRelAggregates`) to its host, or contained in its storey when it has none;
import reads the same back, with the host found by the aggregation. The 3D view draws a tube along each straight piece
(`rebar::tubes`, built with a round run's tube), which the concrete round a bar hides: look at bars with their host hidden. A section draws each bar it cuts as a
dot over the host's poché (`render::section_in` keeps them out of the host's pieces, which would swallow them); a plan draws them too (`render::bar_in_plan`).

Importing a structural program's IFC (Eberick, Cype and the like; tests in `eberick_tests.rs`, written by hand, never from anyone's project). Each
product is named by its mark, so a name that is not its type's (BIMCraft names a product by its type, and writes it as its object type too) is its
`Mark` parameter, for the categories in `import::MARKED` (beams, columns, structural columns, footings, strip footings and floors; not stairs,
which BIMCraft calls "Stair"). The single values of every property set that is not BIMCraft's (`BIMCraft_*`, `RoomSeparation`; `Project Parameters`
is read by its names as written) are project parameters named for the property, a character a formula can't say turned into a space
(`parameter_name`: "Cargas de parede (G2)" is "Cargas de parede G2"), lengths in the file's unit made millimetres; `LoadBearing` is left out because
it already makes a column or a floor structural. The cover Eberick sets (`Cobrimento`, `ConcreteCover`) is therefore on the host as a length, for the
rules that will arm it. A run is found in its body as the way it spreads furthest, except where the body is one extruded profile and the extrusion is
not up: then it runs along the extrusion (`Reader::extrusion`), because a beam between two columns can be shallower than it is deep, and its body then
spreads furthest up. A strip footing, a plan outline stood up, is still read by its shape. A landing slab that belongs to no stair is a floor.

Bars by rules (`reinforce.rs`, the `reinforce` command; tests in `reinforce_tests.rs`). The engineer says how many bars, how thick and how far
apart; the module lays them out in the host's own frame (`Frame`: along, across, up) inside the cover and checks that what is asked fits, with the
message saying what to change. It holds geometry and no standard's table: a spacing or a count is never decided here (that is design), and the
clearance between bars (`clearance`, their thickness and at least 20 mm unless told) is the only rule it brings. Parameters are read with serde types
that deny unknown fields (`Row`, `Pitch`, `Stirrup`, `ColumnBars`, `Mesh`, `Grid`), by key, so a key that is not the host's is refused with the host named.
The cover is `{top, bottom, sides}` (Revit's faces: the top and bottom faces of a beam, a footing or a floor, the ends of a column, and all round
the rest) or one number, and where none is given the host's `Cobrimento` or `ConcreteCover` parameter (a length in mm, as the import of an Eberick IFC
leaves it) is the cover. A spacing is the most between bars: `spread` and `stations` place them evenly from end to end, so the count follows the length
(Revit's Maximum Spacing) and `ends` gives closer stirrups in the first and last `length` mm. A stirrup is a closed loop; with `hook` it runs on past
its first corner to a tail turned into the section at each end (`stirrup`). A beam has `bottom` and `top` rows, `stirrup`, `skin` and `anchor`; a
column `bars` (`x` and `y` along each face, corners counted; a round one, `count` round its circle), `stirrup` (a round one's a polygon of 24 sides), `ties`
(across a rectangular section; with `hook` a tail at each end turned toward the other axis) and `anchor`; a footing a `mesh` of two layers with `hook`
turning their ends up (a round one's bars stop at its circle, `half` in `footing`); a floor `bottom` and `top` meshes (`hook` turning their ends up from the
bottom one and down from the top one, square to the slab's top) whose bars stop a cover short of its outline and of its
openings (`chords`, a scan line through the outline and the openings together; a bar parallel to an opening's edge and nearer it than the cover keeps
its cover from the ends only). A sloped floor's layers are measured from its top face, square to it (`off`, and its secant for the height plumb), so the
cover and the thickness are the type's and its bars follow the slope; a mesh on a slope is one plane's bars. Round beams and strip footings are refused. Each bar is a `rebar` element of the host with a `layer`
(`bottom`, `top`, `skin`, `stirrup`, `bars`, `tie`, `mesh x`, `mesh y`, `bottom x`, `top y`, ...); reinforcing a host again replaces its bars that have a layer
and leaves the ones with none, which were drawn by hand, and one command makes up to `rebar::MAX_BARS`. A bar's layer is in the project file and the
schedule (a field a bar schedule may show), not in IFC. What it does not do: bends other than a hook, laps and splices, bars of a round beam, and any check
against a standard. A section shows a bar in its plane as a strip along it and one it crosses as a dot (the cut of the bar's tube, `section_in`); a bar
beyond the cut stays hidden by the concrete it is in.

More of `reinforce`. `hosts` arms a list (`rebar::MAX_HOSTS`, 1000) alike in one command: every host's bars are made before any is replaced, so one that refuses (its refusal names its category and id) arms none, and `rebar::MAX_BARS_ALL` (20000) caps all of them
together. A `hook` on a beam's `bottom` and `top` rows is the straight leg in mm turned up from the bottom bars and down from the top ones at each end
(a number for both, or `{start, end}`), and on a column's `bars` a leg turned into the section along the way across the face the bar is on
(`{below, above}`); `legs` reads it, with its own words when it is not a length, and a leg that does not stay inside the stirrup (a column's: stop
short of the middle) is refused. The bends are sharp, as every bar's are, and each leg is a piece of the path, so the length and the weight count it.
A `reinforcement_preset` is `reinforce::Preset`, a kind of host (`KINDS`) and the parameters that kind takes, in `Model::reinforcement` and as a library
entry (`Kind::ReinforcementPreset`, so it has an id and a GUID, comes in with Insert from File and is renamed with `rename_entry`); it is checked
for its keys and its size when saved (`Preset::check`, `MAX_PRESETS`) and against a host only when it arms one. `reinforce` with `preset` takes its parameters
and lets any given with it replace them, key by key; a preset of another kind than the host is refused, saying both. Plans: a bar of the plan's level
is drawn from above, solid when the cut plane is over its lowest point and dashed when it is over the plane and under the view range's top, a bar
that stands (a column's) as a dot, and bars that lie on one another (a column's stirrups, one over the next) once.

Bends beyond a hook. `bottom.bent` `{count, angle?, top?}` (`Bent`, `bent_up`) takes `count` of a beam's inner bottom bars (the two at the corners hold the
stirrups and stay straight; the others are picked evenly) and bends them up at each end: along the top face for `top` mm from the end (a fifth of the
length unless told), down a slope of `angle` degrees (15 to 75, 45 unless told) to the bottom row's level and along it. They are on the layer `bent`,
have no hook, count as bottom steel in the check, and are refused where the slopes meet in the middle, where the section is too shallow to rise
in, and where one comes within the clearance of a top bar at the top. `bars.jog` `{by, from_top, slope?}` (`Jog`, `step`, `stepped`) steps a column's
bars `by` mm toward the axis (a corner bar along both faces, a face bar across its own, a round column's radially), the step starting `from_top` mm below
the top and running `slope` mm up for each mm across (6 unless told); the points of the bar above the step move with it, including a hook's leg, and
the bars at their new places are checked against one another for the clearance.

The check (`reinforce/norm.rs`). `check` `"NBR 6118"`, with `aggressiveness` (1 to 4) and `fck` (MPa), makes `reinforce` return `notes`: where the bars
made fall short of the usual limits for the kind of host (Table 7.2's cover by class; for a beam the 4 % most, the 0.15 % least up to C30, skin bars over 600 mm
deep, stirrups of 5 mm to a tenth of the width at most min(0.6 d, 300) apart; for a column bars of 10 mm to an eighth of the side, 4 bars or 6 round,
centres at most min(2 sides, 400), 0.4 to 8 % of steel, stirrups of 5 mm or a quarter of the bars' at most min(200, the side, 12 bars) apart; for a floor
bars of at most an eighth of the thickness at most min(2 h, 200) apart). A note never refuses: an engineer may know better, and the bars are made.
It reads only the bars made, so a limit that needs shear, the load or the aggregate is not in it. The numbers are typed from handbooks and course material
that print them, not from the standard's text, which is paid for, and the item numbers are the 2014 edition's at the level of the chapter's item: check
one against a copy before relying on it. The desktop's Check row (Standard, Aggressiveness class, fck) shows the notes inside the dialog, which stays
open, with the bars made, so a field can change and OK remake them.

The Reinforce dialog (`crates/ui-egui/src/reinforce.rs`, Structure > Reinforcement, RF; tests in `tests/reinforce.rs`). It opens on the one beam, column,
structural column, footing or floor selected (anything else says what to select), with the form of its kind (`BEAM`, `COLUMN`, `FOOTING`, `FLOOR`: tables of
groups of fields, each field the path of the parameter it sets, such as `bottom.count`). Every field is text: a count is a whole number, a bar size is mm,
a length is read in the project's units (`Units::parse`); a field left empty is not sent, a group with none filled is left out, and the command is the one
that says what is missing. OK builds the parameters and runs `reinforce` as one undo step; a refusal is the command's own words, shown inside the dialog
(a status box in the view's corner could lie over its buttons) with the dialog left open and the project unchanged, and a field that is not a number is
named before the command is asked. It takes any number of elements selected, of one form (a mixed selection says so), and arms them with one `hosts` command; Remove Bars removes the bars of all. What was typed is kept for the next element of the same form (not in the project, not on disk). The Preset row lists the project's presets of the form: choosing one fills the fields (`ReinforceDialog::load`, the reverse of `params`, lengths in the project's units), Save Preset keeps the fields as they are under the name typed (`reinforcement_preset`, the parameters without the hosts), and Delete Preset takes the chosen one away; each keeps the dialog open. Remove Bars, shown
when the element has bars the command made (those with a layer), deletes them and leaves the ones drawn by hand. The fields' accessible names are
"Group: Field" (`Bottom bars: Count`), and the cover's is `Cover`, its label having a colon so that no two nodes carry the name.

**Connectors.** A family's `connectors` (`family::Connector`, a list for any discipline; today the electrical device's) say what
a placed device draws: `kind` `power` or `switch`, `system` `power-balanced`, `power-unbalanced` or `switch`, `poles` (1 to 3),
`voltage`, `load_va`, `power_factor` and a `classification`. The numbers are numbers or formulas over the family's parameters, so a
type sets its load (the starter luminaires' `Load` parameter) and an instance parameter lets one component set its own; a
`power-unbalanced` connector's `load_va` may be a list, one load for each pole, and a single number is shared out evenly.
`Family::check` refuses a family of two `power` connectors, a kind and system that don't go together, poles, voltage, power factor
or load out of range for any type, and a load on a switch. A connector is a part (`HasPart`: an id and a GUID, kept by place
while the family is edited), and a family without any reads and places as it always did. A component's `element_json` carries
`connectors` worked out for its type (`Family::connector_values`: `load_va` in all, `load_va_by_phase`), which a circuit will sum;
there are no circuits, panels or fitting-to-pipe joins yet, and the connector holds no position. `Model::warnings` (shown by
`inspect`; `Model::check` stays an error only) says which lighting, outlet or switch family in use holds no connector, so its devices
count as zero load. IFC writes each device with a connector `Pset_ElectricalDeviceCommon` (`NumberOfPoles`, and for a power
connector `PowerFactor` and `RatedVoltage`) and `BIMCraft_Connector` (`System`, `Classification`, `LoadVA`, `LoadVAPhase1`…); IFC
import doesn't read them back.
