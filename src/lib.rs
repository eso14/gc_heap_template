#![cfg_attr(not(test), no_std)]

use core::ops::{Index, IndexMut};

use gc_headers::{GarbageCollectingHeap, HeapError, Pointer, Tracer};

fn independent_elements_from<T>(i: usize, j: usize, slice: &mut [T]) -> Option<(&mut T, &mut T)> {
    if i == j || i >= slice.len() || j >= slice.len() {
        None
    } else if i < j {
        let (left, right) = slice.split_at_mut(j);
        Some((&mut left[i], &mut right[0]))
    } else {
        let (left, right) = slice.split_at_mut(i);
        Some((&mut right[0], &mut left[j]))
    }
}

#[derive(Copy, Clone, Debug)]
struct BlockInfo {
    start: usize,
    size: usize,
    num_times_copied: usize,
}

#[derive(Copy, Clone, Debug)]
struct BlockTable<const MAX_BLOCKS: usize> {
    block_info: [Option<BlockInfo>; MAX_BLOCKS],
}

impl<const MAX_BLOCKS: usize> Index<usize> for BlockTable<MAX_BLOCKS> {
    type Output = Option<BlockInfo>;

    fn index(&self, index: usize) -> &Self::Output {
        &self.block_info[index]
    }
}

impl<const MAX_BLOCKS: usize> IndexMut<usize> for BlockTable<MAX_BLOCKS> {
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.block_info[index]
    }
}

impl<const MAX_BLOCKS: usize> BlockTable<MAX_BLOCKS> {
    fn new() -> Self {
        Self {
            block_info: [None; MAX_BLOCKS],
        }
    }

    fn available_block(&self) -> Option<usize> {
        self.block_info.iter().position(|block| block.is_none())
    }

    fn blocks_in_use(&self) -> impl Iterator<Item = usize> + '_ {
        (0..MAX_BLOCKS).filter(|b| self.block_info[*b].is_some())
    }

    fn blocks_num_copies(&self) -> impl Iterator<Item = (usize, usize)> + '_ {
        self.blocks_in_use()
            .map(|b| (b, self.block_info[b].unwrap().num_times_copied))
    }

    fn address(&self, p: Pointer) -> anyhow::Result<usize, HeapError> {
        let block_num = p.block_num();
        let offset = p.offset();

        if block_num >= MAX_BLOCKS{
            return Err(HeapError::IllegalBlock(block_num, MAX_BLOCKS-1));
        }
        match self.block_info[block_num] {
            None => Err(HeapError::UnallocatedBlock(block_num)),
            Some(info) => {

                if offset >= info.size {
                    return Err(HeapError::OffsetTooBig(offset, block_num ,info.size ));
                }
               
                if p.len() != info.size { 
                    return Err(HeapError::MisalignedPointer(p.len(), info.size, block_num)); 
                }
             
                Ok(info.start + offset)
            }
        }

    }

    fn allocated_block_ptr(&self, block: usize) -> Option<Pointer> {
        match self.block_info.get(block) {
            None => None,
            Some(info) => info.map(|info| Pointer::new(block, info.size)),
        }
    }
}

#[derive(Copy, Clone, Debug)]
struct RamHeap<const HEAP_SIZE: usize> {
    heap: [u64; HEAP_SIZE],
    next_address: usize,
}

impl<const HEAP_SIZE: usize> RamHeap<HEAP_SIZE> {
    fn new() -> Self {
        Self {
            heap: [0; HEAP_SIZE],
            next_address: 0,
        }
    }

    fn clear(&mut self) {
        self.next_address = 0;
    }

    fn load(&self, address: usize) -> anyhow::Result<u64, HeapError> {
        
        if address >= self.next_address {
            return Err(HeapError::IllegalAddress(address, self.next_address));
        }
        Ok(self.heap[address])
    }

    fn store(&mut self, address: usize, value: u64) -> anyhow::Result<(), HeapError> {
        if address >= self.next_address {
            return Err(HeapError::IllegalAddress(address, self.next_address));
        }
        self.heap[address] = value;
        Ok(())
    }

    fn malloc(&mut self, num_words: usize) -> anyhow::Result<usize, HeapError> {
        if num_words == 0 {
            return Err(HeapError::ZeroSizeRequest);
        }
    
        let start = self.next_address;
        let end = start + num_words;
    
        if end > HEAP_SIZE {
            return Err(HeapError::OutOfMemory);
        }
    
        self.next_address = end;
        Ok(start)
        
    }

    fn copy(&self, src: &BlockInfo, dest: &mut Self) -> anyhow::Result<BlockInfo, HeapError> {
        let new_start = dest.malloc(src.size)?;

    for i in 0..src.size {
        let value = self.load(src.start + i)?;
        dest.store(new_start + i, value)?;
    }

    Ok(BlockInfo {
        start: new_start,
        size: src.size,
        num_times_copied: src.num_times_copied + 1,
    })
       
    }
}

pub struct OnceAndDoneHeap<const HEAP_SIZE: usize, const MAX_BLOCKS: usize> {
    heap: RamHeap<HEAP_SIZE>,
    block_info: BlockTable<MAX_BLOCKS>,
}

impl<const HEAP_SIZE: usize, const MAX_BLOCKS: usize> GarbageCollectingHeap
    for OnceAndDoneHeap<HEAP_SIZE, MAX_BLOCKS>
{
    fn new() -> Self {
        Self {
            heap: RamHeap::new(),
            block_info: BlockTable::new(),
        }
    }

    fn address(&self, p: Pointer) -> anyhow::Result<usize, HeapError> {
        self.block_info.address(p)
    }

    fn load(&self, p: Pointer) -> anyhow::Result<u64, HeapError> {
        self.block_info
            .address(p)
            .and_then(|address| self.heap.load(address))
    }

    fn store(&mut self, p: Pointer, value: u64) -> anyhow::Result<(), HeapError> {
        self.block_info
            .address(p)
            .and_then(|address| self.heap.store(address, value))
    }

    fn blocks_in_use(&self) -> impl Iterator<Item = usize> {
        self.block_info.blocks_in_use()
    }

    fn allocated_block_ptr(&self, block: usize) -> Option<Pointer> {
        self.block_info.allocated_block_ptr(block)
    }

    fn blocks_num_copies(&self) -> impl Iterator<Item = (usize, usize)> {
        self.block_info.blocks_num_copies()
    }

    fn malloc<T: Tracer>(&mut self, num_words: usize, _: &T) -> anyhow::Result<Pointer, HeapError> {
        match self.block_info.available_block() {
            Some(block_num) => {
                let start = self.heap.malloc(num_words)?;
                self.block_info[block_num] = Some(BlockInfo {
                    start,
                    size: num_words,
                    num_times_copied: 0,
                });
                Ok(Pointer::new(block_num, num_words))
            }
            None => Err(HeapError::OutOfBlocks),
        }
    }

    fn assert_no_strays(&self) {}
}

pub struct CopyingHeap<const HEAP_SIZE: usize, const MAX_BLOCKS: usize> {
    heaps: [RamHeap<HEAP_SIZE>; 2],
    block_info: BlockTable<MAX_BLOCKS>,
    active_heap: usize,
}

impl<const HEAP_SIZE: usize, const MAX_BLOCKS: usize> CopyingHeap<HEAP_SIZE, MAX_BLOCKS> {
    fn collect<T: Tracer>(&mut self, tracer: &T) -> anyhow::Result<(), HeapError> {
          
          let inactive = (self.active_heap + 1) % 2;
          let (src, dest) =
              independent_elements_from(self.active_heap, inactive, &mut self.heaps).unwrap();
        let mut blocks_used = [false; MAX_BLOCKS];
        tracer.trace(&mut blocks_used);

        let mut new_block_info = [None; MAX_BLOCKS];

        for (i, &in_use) in blocks_used.iter().enumerate() {
            if in_use {
                if let Some(block) = &self.block_info[i] {
                    let copied = src.copy(block, dest)?;
                    new_block_info[i] = Some(copied);
                }
            }
        }

    
        for block in src.heap.iter_mut() {
            *block = 0; 
        }

        src.clear();

        self.block_info.block_info = new_block_info;
        self.active_heap = inactive;

        Ok(())

       
          // Outline
          //
          // 1. Run the `trace()` method of the `tracer` to find blocks in use.
          // 2. For each block in use:
          //    * Copy the block from `src` to `dest`.
          // 3. Clear the active heap.
          // 4. Set `self.active_heap` to point at the newly active heap.
    }
}

impl<const HEAP_SIZE: usize, const MAX_BLOCKS: usize> GarbageCollectingHeap
    for CopyingHeap<HEAP_SIZE, MAX_BLOCKS>
{
    fn new() -> Self {
        Self {
            heaps: [RamHeap::new(); 2],
            block_info: BlockTable::new(),
            active_heap: 0,
        }
    }

    fn address(&self, p: Pointer) -> anyhow::Result<usize, HeapError> {
        self.block_info.address(p)
    }

    fn load(&self, p: Pointer) -> anyhow::Result<u64, HeapError> {
        
        
        self.block_info
            .address(p)
            .and_then(|address| self.heaps[self.active_heap].load(address))
    }

    fn store(&mut self, p: Pointer, value: u64) -> anyhow::Result<(), HeapError> {
        self.block_info
            .address(p)
            .and_then(|address| self.heaps[self.active_heap].store(address, value))
    }

    fn blocks_in_use(&self) -> impl Iterator<Item = usize> {
        self.block_info.blocks_in_use()
    }

    fn allocated_block_ptr(&self, block: usize) -> Option<Pointer> {
        self.block_info.allocated_block_ptr(block)
    }

    fn blocks_num_copies(&self) -> impl Iterator<Item = (usize, usize)> {
        self.block_info.blocks_num_copies()
    }

    fn malloc<T: Tracer>(
        &mut self,
        num_words: usize,
        tracer: &T,
    ) -> anyhow::Result<Pointer, HeapError> {
        if num_words == 0{
            return Err(HeapError::ZeroSizeRequest);
        }

        let block_num = match self.block_info.available_block() {
            Some(b) => b,
            None => {
                self.collect(tracer)?;
                self.block_info.available_block().ok_or(HeapError::OutOfBlocks)?
            }
        };
    
       
        let start = match self.heaps[self.active_heap].malloc(num_words) {
            Ok(s) => s,
            Err(HeapError::OutOfMemory) => {
                self.collect(tracer)?;
                self.heaps[self.active_heap].malloc(num_words).map_err(|_| HeapError::OutOfMemory)?
            }
            Err(e) => return Err(e),
        };
    
       
        self.block_info[block_num] = Some(BlockInfo {
            start,
            size: num_words,
            num_times_copied: 0,
        });
    
        Ok(Pointer::new(block_num, num_words))

    }

    fn assert_no_strays(&self) {
        assert!(self.heaps[(self.active_heap + 1) % 2].next_address == 0);
    }
}

#[derive(Copy, Clone, Debug)]
pub struct GenerationalHeap<
    const HEAP_SIZE: usize,
    const MAX_BLOCKS: usize,
    const MAX_COPIES: usize,
> {
    gen_0: [RamHeap<HEAP_SIZE>; 2],
    gen_1: [RamHeap<HEAP_SIZE>; 2],
    block_info: BlockTable<MAX_BLOCKS>,
    active_gen_0: usize,
    active_gen_1: usize,
}

impl<const HEAP_SIZE: usize, const MAX_BLOCKS: usize, const MAX_COPIES: usize>
    GenerationalHeap<HEAP_SIZE, MAX_BLOCKS, MAX_COPIES>
{
    fn active_inactive_gen_0_gen_1(
        &mut self,
    ) -> (
        &mut RamHeap<HEAP_SIZE>,
        &mut RamHeap<HEAP_SIZE>,
        &mut RamHeap<HEAP_SIZE>,
        &mut RamHeap<HEAP_SIZE>,
    ) {
        let inactive_0 = (self.active_gen_0 + 1) % 2;
        let inactive_1 = (self.active_gen_1 + 1) % 2;
        let (active_0, inactive_0) =
            independent_elements_from(self.active_gen_0, inactive_0, &mut self.gen_0).unwrap();
        let (active_1, inactive_1) =
            independent_elements_from(self.active_gen_1, inactive_1, &mut self.gen_1).unwrap();
        (
            active_0,
            inactive_0,
            active_1,
            inactive_1,
        )
    }
    fn heap_and_gen_for(&self, block_num: usize) -> anyhow::Result<(usize, usize), HeapError> {
        if block_num >= MAX_BLOCKS {
            Err(HeapError::IllegalBlock(block_num, MAX_BLOCKS - 1))
        } else {
            match self.block_info[block_num] {
                Some(block_info) => Ok(if block_info.num_times_copied > MAX_COPIES {
                    (self.active_gen_1, 1)
                } else {
                    (self.active_gen_0, 0)
                }),
                None => Err(HeapError::UnallocatedBlock(block_num)),
            }
        }
    }

    fn collect_gen_0<T: Tracer>(&mut self, tracer: &T) -> anyhow::Result<(), HeapError> {
        let mut new_block_info = self.block_info.clone();

  
        let (active_0, inactive_0, active_1, inactive_1) =
            self.active_inactive_gen_0_gen_1();

        let mut blocks_used = [false; MAX_BLOCKS];
        tracer.trace(&mut blocks_used);

        let mut gen_1_collected = false;

        for (i, &in_use) in blocks_used.iter().enumerate() {
            if in_use {
                if let Some(block) = new_block_info[i] {
                    let block_size = block.size;
                    let src_start = block.start;

                    if block.num_times_copied == MAX_COPIES {
                        let dest_start = match active_1.malloc(block_size) {
                            Ok(addr) => addr,
                            Err(_) => {
                                if gen_1_collected {
                                    return Err(HeapError::OutOfMemory);
                                }

                                Self::collect_gen_1(&blocks_used, &mut new_block_info, active_1, inactive_1)?;
                                gen_1_collected = true;

                                
                                inactive_1.malloc(block_size)?
                            }
                        };

                        for j in 0..block_size {
                            let value = active_0.load(src_start + j)?;
                            inactive_1.store(dest_start + j, value)?;
                        }

                        new_block_info[i] = Some(BlockInfo {
                            start: dest_start,
                            size: block_size,
                            num_times_copied: block.num_times_copied + 1,
                        });
                    } else {
                        let dest_start = inactive_0.malloc(block_size)?;
                        for j in 0..block_size {
                            let value = active_0.load(src_start + j)?;
                            inactive_0.store(dest_start + j, value)?;
                        }

                        new_block_info[i] = Some(BlockInfo {
                            start: dest_start,
                            size: block_size,
                            num_times_copied: block.num_times_copied + 1,
                        });
                    }
                }
            }
        }

   
        


        active_0.clear();
        self.active_gen_0 = (self.active_gen_0 + 1) % 2;

   
        if gen_1_collected {
            self.active_gen_1 = (self.active_gen_1 + 1) % 2;
        }
        self.block_info = new_block_info;

        Ok(())

         // Outline
        //
        // 1. Call the tracer to find out what blocks are in use.
        // 2. For each block in use:
        //    * If it has been copied MAX_COPIES times
        //      * You'll need a variable to track whether you have already performed a generation 1 collection.
        //      * If so, just return the error - multiple generation 1 collections will not be productive
        //      * If not, copy into the active generation 1 heap.
        //      * If that heap is out of space, perform a generation 1 collection.
        //      * After the generation 1 collection, try copying it into the inactive generation 1 heap.
        //    * If not, copy it into the inactive generation 0 heap.
        // 3. Clear the active generation 0 heap.
        // 4. Update self.active_gen_0 to the other heap.
        // 5. If there was a generation 1 collection, update self.active_gen_1 to the other heap.
    }

    fn collect_gen_1(
        blocks_used: &[bool; MAX_BLOCKS],
        block_info: &mut BlockTable<MAX_BLOCKS>,
        src: &mut RamHeap<HEAP_SIZE>,
        dest: &mut RamHeap<HEAP_SIZE>,
    ) -> anyhow::Result<(), HeapError> {
        for (i, &in_use) in blocks_used.iter().enumerate() {
            if in_use {
                if let Some(block) = &block_info[i] {
                    if block.num_times_copied > MAX_COPIES {
                        let block_size = block.size;
                        let src_start = block.start;

                        let dest_start = dest.malloc(block_size)?;
                        for j in 0..block_size {
                            let value = src.load(src_start + j)?;
                            dest.store(dest_start + j, value)?;
                        }

                        block_info[i] = Some(BlockInfo {
                            start: dest_start,
                            size: block_size,
                            num_times_copied: block.num_times_copied + 1,
                        });
                    }
                }
            }
        }


        src.clear();

        Ok(())
         // Outline
        //
        // 1. For each block in use:
        //    * If it has been copied more than MAX_COPIES times, copy it to `dest`
        // 2. Clear the `src` heap.
    }
}


impl<const HEAP_SIZE: usize, const MAX_BLOCKS: usize, const MAX_COPIES: usize> GarbageCollectingHeap
    for GenerationalHeap<HEAP_SIZE, MAX_BLOCKS, MAX_COPIES>
{
    fn new() -> Self {
        Self {
            gen_0: [RamHeap::new(); 2],
            gen_1: [RamHeap::new(); 2],
            block_info: BlockTable::new(),
            active_gen_0: 0,
            active_gen_1: 0,
        }
    }

    fn load(&self, p: Pointer) -> anyhow::Result<u64, HeapError> {
        let (heap, gen) = self.heap_and_gen_for(p.block_num())?;
        let address = self.block_info.address(p)?;
        (if gen == 0 {
            &self.gen_0[heap]
        } else {
            &self.gen_1[heap]
        })
        .load(address)
    }

    fn store(&mut self, p: Pointer, value: u64) -> anyhow::Result<(), HeapError> {
        let (heap, gen) = self.heap_and_gen_for(p.block_num())?;
        let address = self.block_info.address(p)?;
        (if gen == 0 {
            &mut self.gen_0[heap]
        } else {
            &mut self.gen_1[heap]
        })
        .store(address, value)
    }

    fn address(&self, p: Pointer) -> anyhow::Result<usize, HeapError> {
        self.block_info.address(p)
    }

    fn blocks_in_use(&self) -> impl Iterator<Item = usize> {
        self.block_info.blocks_in_use()
    }

    fn allocated_block_ptr(&self, block: usize) -> Option<Pointer> {
        self.block_info.allocated_block_ptr(block)
    }

    fn blocks_num_copies(&self) -> impl Iterator<Item = (usize, usize)> {
        self.block_info.blocks_num_copies()
    }

    fn malloc<T: Tracer>(
        &mut self,
        num_words: usize,
        tracer: &T,
    ) -> anyhow::Result<Pointer, HeapError> {
        let mut free_block_num = None;

    for i in 0..MAX_BLOCKS {
        if self.block_info[i].is_none() {
            free_block_num = Some(i);
            break;
        }
    }

    if free_block_num.is_none() {
        self.collect_gen_0(tracer)?;
        for i in 0..MAX_BLOCKS {
            if self.block_info[i].is_none() {
                free_block_num = Some(i);
                break;
            }
        }
        if free_block_num.is_none() {
            return Err(HeapError::OutOfBlocks);
        }
    }

    let block_num = free_block_num.unwrap();
    
    let addr = match self.gen_0[self.active_gen_0].malloc(num_words) {
        Ok(addr) => addr,
        Err(_) => {
            self.collect_gen_0(tracer)?;
            self.gen_0[self.active_gen_0].malloc(num_words)?
        }
    };

    self.block_info[block_num] = Some(BlockInfo {
        start: addr,
        size: num_words,
        num_times_copied: 0,
    });

    Ok(Pointer::new(block_num, num_words))
        // Outline
        //
        // 1. Find an available block number
        //    * If none are available, perform a collection.
        //    * If none are still available, report out of blocks.
        // 2. Perform a generation zero malloc.
        //    * If no space is available, perform a collection.
        //    * If no space is still available, report out of memory.
        // 3. Create entry in the block table for the newly allocated block.
        // 4. Return a pointer to the newly allocated block.
    }

    fn assert_no_strays(&self) {
        assert!(self.gen_0[(self.active_gen_0 + 1) % 2].next_address == 0);
        assert!(self.gen_1[(self.active_gen_1 + 1) % 2].next_address == 0);
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use core::fmt::Debug;

    use super::*;
    use test_tracer::TestTracer;

    const HEAP_SIZE: usize = 96;
    const MAX_BLOCKS: usize = 12;

    // Level 1 Unit Tests

    #[test]
    fn basic_allocation_test() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = OnceAndDoneHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
    }

    #[test]
    fn out_of_blocks_test() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = OnceAndDoneHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_out_of_blocks(&mut allocator, &mut tracer);
    }

    #[test]
    fn test_bad_address_error() {
        let mut allocator = RamHeap::<HEAP_SIZE>::new();
        match allocator.load(HEAP_SIZE + 1) {
            Ok(_) => panic!("This should have been an IllegalAddress error."),
            Err(e) => assert_eq!(e, HeapError::IllegalAddress(HEAP_SIZE + 1, 0))
        }

        allocator.malloc(96).unwrap();
        match allocator.load(HEAP_SIZE + 1) {
            Ok(_) => panic!("This should have been an IllegalAddress error."),
            Err(e) => assert_eq!(e, HeapError::IllegalAddress(HEAP_SIZE + 1, HEAP_SIZE))
        }
    }

    // Level 2 Unit Tests

    #[test]
    fn deallocation_test() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
        allocator.assert_no_strays();
        test_out_of_blocks(&mut allocator, &mut tracer);
        test_remove_half(&mut allocator, &mut tracer, &mut blocks2ptrs);
    }

    #[test]
    fn collection_test() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_out_of_blocks(&mut allocator, &mut tracer);
        test_remove_half(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_force_collection(&mut allocator, &mut tracer, &mut blocks2ptrs);
        allocator.assert_no_strays();
    }

    #[test]
    fn full_test() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_remove_half(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_force_collection(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_fill_ram(&mut allocator, &mut tracer, &mut blocks2ptrs);
        allocator.assert_no_strays();
        test_out_of_ram(&mut allocator, &mut tracer);
    }

    #[test]
    fn test_no_blocks_error() {
        let mut blocks2ptrs = HashMap::new();
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        test_initial_allocation(&mut allocator, &mut tracer, &mut blocks2ptrs);
        test_out_of_blocks(&mut allocator, &mut tracer);
    }

    #[test]
    fn test_zero_size_error() {
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let tracer = TestTracer::default();
        match allocator.malloc(0, &tracer) {
            Ok(_) => panic!("This should have been a zero-size error"),
            Err(e) => assert_eq!(e, HeapError::ZeroSizeRequest),
        }
    }

    #[test]
    fn test_illegal_block_error() {
        let allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let bad_ptr = Pointer::new(MAX_BLOCKS, 1);
        match allocator.load(bad_ptr) {
            Ok(_) => panic!("This should have been an error"),
            Err(e) => assert_eq!(e, HeapError::IllegalBlock(MAX_BLOCKS, MAX_BLOCKS - 1))
        }
    }

    #[test]
    fn test_unallocated_block_error() {
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let tracer = TestTracer::default();
        let p = allocator.malloc(1, &tracer).unwrap();
        let bad_ptr = Pointer::new(p.block_num() + 1, 1);
        match allocator.load(bad_ptr) {
            Ok(_) => panic!("This should have been an UnallocatedBlock error"),
            Err(e) => assert_eq!(e, HeapError::UnallocatedBlock(bad_ptr.block_num()))
        }
    }

    #[test]
    fn test_offset_error() {
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        let p = tracer.allocate_next(HEAP_SIZE, &mut allocator).unwrap();
        let s = p.iter().skip(1).next().unwrap();
        tracer.deallocate_next().unwrap();
        tracer.allocate_next(1, &mut allocator).unwrap();
        let q = tracer.allocate_next(1, &mut allocator).unwrap();
        assert_eq!(p.block_num(), q.block_num());
        match allocator.load(s) {
            Ok(_) => panic!("This should have been an OffsetTooBig error"),
            Err(e) => assert_eq!(e, HeapError::OffsetTooBig(1, p.block_num(), 1))
        }
    }

    #[test]
    fn test_misaligned_pointer_error() {
        let mut allocator = CopyingHeap::<HEAP_SIZE, MAX_BLOCKS>::new();
        let mut tracer = TestTracer::default();
        let p = tracer.allocate_next(HEAP_SIZE, &mut allocator).unwrap();
        tracer.deallocate_next().unwrap();
        tracer.allocate_next(1, &mut allocator).unwrap();
        let q = tracer.allocate_next(1, &mut allocator).unwrap();
        assert_eq!(p.block_num(), q.block_num());
        match allocator.load(p) {
            Ok(_) => panic!("This should have been a MisalignedPointer error"),
            Err(e) => assert_eq!(e, HeapError::MisalignedPointer(HEAP_SIZE, 1, p.block_num()))
        }
    }

    fn test_initial_allocation<H: GarbageCollectingHeap>(
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        for (block_num, request) in [2, 10, 4, 8, 6, 12, 6, 24, 4, 8, 2, 8].iter().enumerate() {
            println!("block: {block_num} request: {request}");
            let allocated_ptr = tracer.allocate_next(*request, allocator).unwrap();
            assert_eq!(block_num, allocated_ptr.block_num());
            assert_eq!(*request, allocated_ptr.len());
            blocks2ptrs.insert(block_num, allocated_ptr);
            assert_eq!(blocks2ptrs.len(), allocator.num_allocated_blocks());
            ensure_non_overlapping(blocks2ptrs, allocator);
        }
        ensure_all_match(blocks2ptrs, allocator);
        assert_eq!(total_words_allocated(blocks2ptrs), 94);
        test_load_store(&blocks2ptrs, allocator);
        assert_eq!(allocator.num_allocated_blocks(), 12);
    }

    fn test_remove_half<H: GarbageCollectingHeap>(
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        for _ in 0..(tracer.len() / 2) {
            let removed = tracer.deallocate_next_even().unwrap();
            assert!(blocks2ptrs.contains_key(&removed.block_num()));
            blocks2ptrs.remove(&removed.block_num());
        }
        test_load_store(&blocks2ptrs, allocator);
        assert_eq!(allocator.num_allocated_blocks(), 12);
    }

    fn test_force_collection<H: GarbageCollectingHeap>(
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        let ptr = tracer.allocate_next(4, allocator).unwrap();
        assert!(!blocks2ptrs.contains_key(&ptr.block_num()));
        blocks2ptrs.insert(ptr.block_num(), ptr);
        assert_eq!(allocator.num_allocated_blocks(), 7);
        assert_eq!(tracer.len(), allocator.num_allocated_blocks());
    }

    fn test_fill_ram<H: GarbageCollectingHeap>(
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        let ptr = tracer.allocate_next(68, allocator).unwrap();
        assert!(!blocks2ptrs.contains_key(&ptr.block_num()));
        blocks2ptrs.insert(ptr.block_num(), ptr);
        assert_eq!(allocator.num_allocated_blocks(), 8);
        assert_eq!(tracer.total_allocated(), 96);
    }

    fn test_out_of_ram<H: GarbageCollectingHeap>(allocator: &mut H, tracer: &mut TestTracer) {
        match tracer.allocate_next(1, allocator) {
            Ok(_) => panic!("Should be an out of memory error!"),
            Err(e) => assert_eq!(e, HeapError::OutOfMemory),
        }
    }

    fn ensure_all_match<H: GarbageCollectingHeap>(
        blocks2ptrs: &HashMap<usize, Pointer>,
        allocator: &H,
    ) {
        for (block, ptr) in blocks2ptrs.iter() {
            assert_eq!(allocator.allocated_block_ptr(*block).unwrap(), *ptr);
        }
    }

    fn ensure_non_overlapping<H: GarbageCollectingHeap>(
        blocks2ptrs: &HashMap<usize, Pointer>,
        allocator: &H,
    ) {
        let mut memory_locations = (0..HEAP_SIZE).collect::<HashSet<_>>();
        for ptr in blocks2ptrs.values() {
            for inner in ptr.iter() {
                let addr = allocator.address(inner).unwrap();
                assert!(memory_locations.contains(&addr));
                memory_locations.remove(&addr);
            }
        }
    }

    fn test_load_store<H: GarbageCollectingHeap>(
        blocks2ptrs: &HashMap<usize, Pointer>,
        allocator: &mut H,
    ) {
        let mut value = 0;
        for p in blocks2ptrs.values() {
            for pt in p.iter() {
                allocator.store(pt, value).unwrap();
                assert_eq!(value, allocator.load(pt).unwrap());
                value += 1;
            }
        }

        value = 0;
        for p in blocks2ptrs.values() {
            for pt in p.iter() {
                assert_eq!(value, allocator.load(pt).unwrap());
                value += 1;
            }
        }
    }

    fn total_words_allocated(blocks2ptrs: &HashMap<usize, Pointer>) -> usize {
        blocks2ptrs.values().map(|p| p.len()).sum()
    }

    fn test_out_of_blocks<H: GarbageCollectingHeap>(allocator: &mut H, tracer: &mut TestTracer) {
        match tracer.allocate_next(1, allocator) {
            Ok(_) => panic!("Allocator should be out of space - this should be an error"),
            Err(e) => assert_eq!(e, HeapError::OutOfBlocks),
        }
    }

    // Level 3 Unit Test

    #[test]
    fn generational_test() {
        let mut allocator = GenerationalHeap::<100, 120, 2>::new();
        let mut tracer = TestTracer::default();
        let mut blocks2ptrs = HashMap::new();
        allocate_many(40, &mut allocator, &mut tracer, &mut blocks2ptrs);
        allocator.assert_no_strays();

        assert_eq!(blocks2ptrs.len(), allocator.num_allocated_blocks());
        for (_, c) in allocator.blocks_num_copies() {
            assert_eq!(c, 0);
        }
        
        for expected_copies in 1..=3 {
            force_copy_n(expected_copies, &mut allocator, &mut tracer, &mut blocks2ptrs);
            for (b, c) in allocator.blocks_num_copies() {
                if b >= expected_copies && b < blocks2ptrs.len() {
                    assert_eq!(c, expected_copies);
                }
                if let Some(p) = blocks2ptrs.get(&b) {
                    assert_eq!(p.len() as u64, allocator.load(*p).unwrap());
                }
            }
            allocator.assert_no_strays();
        }

        allocate_many(38, &mut allocator, &mut tracer, &mut blocks2ptrs);
        allocator.assert_no_strays();
        
        for _ in 1..=4 {
            tracer.deallocate_next().unwrap();
            tracer.allocate_next(1, &mut allocator).unwrap();
            allocator.assert_no_strays();
        }

        for (_, c) in allocator.blocks_num_copies() {
            assert!(c <= 3);
        }   

        tracer.deallocate_any_that(|p| p.len() != 3);

        tracer.allocate_next(1, &mut allocator).unwrap();
        allocator.assert_no_strays();
        for (_, c) in allocator.blocks_num_copies() {
            assert!(c <= 4);
        } 
    }

    fn allocate_many<H: GarbageCollectingHeap + Debug>(
        num_allocations: usize,
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        for i in 0..num_allocations {
            let size = i % 4 + 1;
            let p = tracer.allocate_next(size, allocator).unwrap();
            blocks2ptrs.insert(p.block_num(), p);
            for addr in p.iter() {
                allocator.store(addr, size as u64).unwrap();
            }
        }
    }

    fn force_copy_n<H: GarbageCollectingHeap + Debug>(
        n: usize,
        allocator: &mut H,
        tracer: &mut TestTracer,
        blocks2ptrs: &mut HashMap<usize, Pointer>,
    ) {
        let d = tracer.deallocate_next().unwrap();
        assert_eq!(n, d.len());
        blocks2ptrs.remove(&d.block_num());
        let p = tracer.allocate_next(n, allocator).unwrap();
        blocks2ptrs.insert(p.block_num(), p);
        allocator.store(p, n as u64).unwrap();
    }
}
