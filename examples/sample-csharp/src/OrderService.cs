namespace Sample;

/// <summary>
/// Places orders on behalf of a customer. This is the seam an AI agent would
/// most likely touch, so it is a good target for `concede why`.
/// </summary>
public sealed class OrderService
{
    private readonly IOrderRepository _repository;

    public OrderService(IOrderRepository repository)
    {
        _repository = repository;
    }

    public Order Place(string customerId, IEnumerable<OrderLine> lines)
    {
        var order = new Order(customerId);

        foreach (var line in lines)
            order.AddLine(line.Sku, line.Quantity, line.UnitPrice);

        order.Submit();
        _repository.Save(order);
        return order;
    }
}

public interface IOrderRepository
{
    void Save(Order order);
}
