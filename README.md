# Bevy Node Plumber

A set of tools that should help to create custom pipelines and execute it in the Bevy Render world. 
This is an alternative to https://github.com/Kjolnyr/bevy_app_compute but with the significant differences:
* Executed in Bevy Render World as sub-graph.
* Can be configured in runtime.
* Allows to trigger manually.
* Supports dynamic buffers sizes depending on input.
* Supports dynamic workgroups dispatches depending on input. 

# Usage

Add the plugin:
```
use bevy::prelude::*;
use bevy_node_plumber::prelude::*;

app.add_plugins(NodePlumberPlugin);
```

Create pipeline nodes or node Providers. They should be added as components to the main world and will be processed asynchronously by the plugin:

```
let fill_buffer_node = builder::ComputeNodeBuilder::default()
    .shader(asset_server.load("shaders/example_fill_f32_buffer.wgsl"))
    .entry_point("main")
    .dispatch_workgroups_strategy(compute::DispatchWorkgroupsStrategy::FromGraphContext(
        |graph| {
            let x = graph
                .get_input_buffer("buffer")
                .map_or(1, |b| b.size() / size_of::<f32>() as u64); // one group for each f32, 
                                     // probably some encase calculations should be used there
            (x as u32, 1, 1)
        },
    ));
let fill_buffer_node = fill_buffer_node
    .bind_resource()
    .name("buffer") // this will be used as name for the input and/or output
    .binding(0)     // binding index
    .input_output() // The same buffer as input and output
    .buffer()
    .add();
let fill_buffer_node = fill_buffer_node.build().unwrap();
let fill_buffer_entity = commands.spawn(fill_buffer_node.clone()).id();
```

Create an inputs and outputs if required. Inputs and outputs are also components and should be spawned in the main world:

```
let input_buffer = input::StorageBufferNode::default(); // Input storage buffer
let output_buffer = output::OutputBuffer::default();

input_buffer.set(vec![0.0; 65535]); // Fill the buffer from start
input_buffer.add_usages(BufferUsages::COPY_SRC); // Since this buffer will be used as and input and output
```

The trigger is also component, so we can trigger this pipeline from a system:

```
let trigger = graph::SubGraphTrigger::Manual(Arc::new(true.into()));
```

Sub-graph definition. We will put all together there:

```
let sub_graph = builder::SubGraphBuilder::default()
    .name("test_compute_sub_graph".into())              // Will be used as sub-graph name and runner node name
    .add_node("input_buffer", input_buffer.clone())     // Add the input, inner buffer is in Arc<Mutex>
    .add_node("output_buffer", output_buffer.clone())   // Output, same as the input
    // Add the node provider. Will be added to the sub-graph when ready.
    .add_node_provider(
        "fill_buffer_node".into(),
        fill_buffer_entity,
        &fill_buffer_node,
    )
    // Connect the sub-graph input node with the input_buffer node
    .add_node_edge(RenderGraph::INPUT_NODE_NAME, "input_buffer") 
    // Connect slots
    .add_slot_edge(
        "input_buffer",
        input::SLOT_NAME,
        "fill_buffer_node",
        "buffer",
    )
    .add_slot_edge(
        "fill_buffer_node",
        "buffer",
        "output_buffer",
        output::SLOT_NAME,
    )
    .trigger(trigger.clone()) // Install trigger
    .build()
    .unwrap();
```

Spawn the rest of the components as one bundle:

```
commands.spawn((sub_graph, trigger, output_buffer, input_buffer));
```

# Docs

TBD

# License

This repository is dual-licensed under either:
* MIT License (LICENSE-MIT or http://opensource.org/licenses/MIT)
* Apache License, Version 2.0 (LICENSE-APACHE or http://www.apache.org/licenses/LICENSE-2.0)