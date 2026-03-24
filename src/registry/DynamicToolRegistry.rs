use std::collections::HashMap;

pub struct DynamicToolRegistry {
    tools: HashMap<String, Box<dyn Fn() + Send + Sync>>,
}

impl DynamicToolRegistry {
    pub fn new() -> Self {
        DynamicToolRegistry {
            tools: HashMap::new(),
        }
    }

    /**
     * Dynamically registers a new tool at runtime.
     * Allows agents to expand their capabilities without re-deployment.
     */
    pub fn register<F>(&mut self, name: &str, tool: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.tools.insert(name.to_string(), Box::new(tool));
    }
}
