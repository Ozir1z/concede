using Xunit;

namespace Sample.Tests;

public class OrderTests
{
    [Fact]
    public void Submit_moves_a_non_empty_order_to_submitted()
    {
        var order = new Order("cust-1");
        order.AddLine("sku-1", 2, 9.99m);

        order.Submit();

        Assert.Equal(OrderStatus.Submitted, order.Status);
    }

    [Fact]
    public void Submit_rejects_an_empty_order()
    {
        var order = new Order("cust-1");

        Assert.Throws<InvalidOperationException>(order.Submit);
    }

    [Fact]
    public void Submit_is_not_allowed_twice()
    {
        var order = new Order("cust-1");
        order.AddLine("sku-1", 1, 5m);
        order.Submit();

        Assert.Throws<InvalidOperationException>(order.Submit);
    }
}
