# Reflux
Reflux is a cutting-edge Rust library designed to streamline the development of microservices with a focus on scalability, flexibility, and usability. By leveraging Rust's performance and safety features, Reflux empowers developers to build robust, high-performance microservices that can seamlessly adapt to evolving business needs. Whether you're scaling up to handle millions of requests or integrating diverse service components, Reflux provides the tools and framework you need to achieve efficient and maintainable microservice architectures. Dive into Reflux and transform the way you build and manage microservices with ease and confidence.

# Reflux Objects
In Reflux, there are various object types that are available.

 ### Extractor
 The extractor is responsible for reading data from an external source (such as a file or socket connection) and yielding data extracted from the source

 ### Transformer
 The transformer is responsible for mutating data. A transformer can convert data from one type to another, or mutate data, but keep the type.

 The transformer has three behaviours:
  - Mutated data: A transformer returns a 'mutated' enum with the mutated data. This will pass the data along a Reflux pipeline.
  - Incomplete mutation - A transformer returns a 'needs more work' enum with the data in it's original type. This is used to feed the data back into the transformer for further processing. This behaviour is useful for recursive functions, such as walking through a directory tree.
  - Error - A transformer returns an 'error' enum with the error message. 

## Note: 
Reflux is currently unstable, and is subject to change in future releases
