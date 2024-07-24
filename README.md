# Reflux
Reflux is a cutting-edge Rust library designed to streamline the development of microservices with a focus on scalability, flexibility, and usability. By leveraging Rust's performance and safety features, Reflux empowers developers to build robust, high-performance microservices that can seamlessly adapt to evolving business needs. Whether you're scaling up to handle millions of requests or integrating diverse service components, Reflux provides the tools and framework you need to achieve efficient and maintainable microservice architectures. Dive into Reflux and transform the way you build and manage microservices with ease and confidence.

# Use cases
- Pipeline workflows
- Routing
- Load balancing

# Reflux Objects
In Reflux, there are various object types that are available.

 ### Extractor

 ![Extractor](https://github.com/user-attachments/assets/532d89e1-9274-4c7f-9362-1cbcaade428c)
 
 The extractor is responsible for reading data from an external source (such as a file or socket connection) and yielding data extracted from the source.

 ### Transformer

 ![Transformer](https://github.com/user-attachments/assets/74206a56-4f70-4abe-8a89-3e66060d0c4a)

 
 The transformer is responsible for mutating data. A transformer can convert data from one type to another, or mutate data, but keep the type.

 The transformer has three behaviours:
  - Mutated data: A transformer returns a 'mutated' enum with the mutated data. This will pass the data along a Reflux pipeline.
  - Incomplete mutation - A transformer returns a 'needs more work' enum with the data in it's original type. This is used to feed the data back into the transformer for further processing. This behaviour is useful for recursive functions, such as walking through a directory tree.
  - Error - A transformer returns an 'error' enum with the error message.

### Router

![Router](https://github.com/user-attachments/assets/f6e3b881-1e2d-4919-b95b-aea6c581d772)


The router is responsible for routing data amongst a set of `Receiver` s in a Round Robin fasion.

### Filter

![Filter](https://github.com/user-attachments/assets/df3d67d6-6bad-46d9-a506-b19eb0eed322)


The filter is responsible for conditionally allowing data to flow through a `Reflux` pipeline. A predicate is supplied to a `Filter` and if data satisfies the predicate, it may pass through.

### Broadcast

![Broadcast](https://github.com/user-attachments/assets/0c2a4395-4656-40cb-b70c-77b55df1158a)


The `Broadcast` is responsible for broadcasting data to multiple `Sender` s.

### Funnel
The `Funnel` is responsible for collecting data from multiple `Receiver` s and sending the data through to a single `Sender`.

### Messenger
The `Messenger` is responsible for receiving messages and passing it through to the relevant `Sender`.

### Loader
The `Loader` is the end of a `Reflux` pipeline. A loader can drop data, or write it to an external source (such as a file or socket).

## Note: 
Reflux is currently unstable, and is subject to change in future releases
